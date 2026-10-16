//! Portable offline tools with copied owned material definitions and strict relative paths.
use crate::{
    brush_presets::Preset,
    material_library::{Asset, FIRST_ID, Kind, Library as Materials},
    user_tools::Library,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: usize = 12 * 1024 * 1024;
fn bad(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.into())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: String,
    bytes: usize,
    hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    checksum: String,
    tools: File,
    materials: Vec<File>,
}
fn hash(bytes: &[u8]) -> String {
    let mut n = 0xcbf29ce484222325u64;
    for b in bytes {
        n = (n ^ *b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{n:016x}")
}
fn refs(p: &Preset) -> impl Iterator<Item = (u32, Kind)> + '_ {
    p.sequence
        .tips
        .iter()
        .copied()
        .map(|id| (id, Kind::Tip))
        .chain(std::iter::once((p.overlay.source, Kind::Texture)))
        .filter(|(id, kind)| {
            *id > match kind {
                Kind::Tip => 7,
                Kind::Texture => 3,
            }
        })
}
fn remap(p: &mut Preset, map: &BTreeMap<u32, u32>) {
    for id in &mut p.sequence.tips {
        if let Some(next) = map.get(id) {
            *id = *next;
        }
    }
    if let Some(id) = map.get(&p.overlay.source) {
        p.overlay.source = *id;
    }
}
fn metadata(path: &Path) -> io::Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(bad("Bundle reparse points are not allowed"));
        }
    }
    if m.file_type().is_symlink() {
        return Err(bad("Bundle links are not allowed"));
    }
    Ok(m)
}
fn read(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let m = metadata(path)?;
    if !m.is_file() || m.len() > max as u64 {
        return Err(bad("Bundle file size/type limit"));
    }
    let mut bytes = vec![];
    fs::File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(bad("Bundle file grew beyond limit"));
    }
    Ok(bytes)
}
fn asset_id(path: &str) -> Option<u32> {
    let middle = path.strip_prefix("materials/")?.strip_suffix(".json")?;
    let id = middle.parse::<u32>().ok()?;
    (id >= FIRST_ID && middle == id.to_string()).then_some(id)
}
fn checked(root: &Path, file: &File, max: usize) -> io::Result<Vec<u8>> {
    let path = root.join(&file.path);
    if file.path != "tools.json" && asset_id(&file.path).is_none() {
        return Err(bad("Invalid bundle relative path"));
    }
    if !path.canonicalize()?.starts_with(root) {
        return Err(bad("Bundle path escaped root"));
    }
    let bytes = read(&path, max)?;
    if file.bytes != bytes.len() || file.hash != hash(&bytes) {
        return Err(bad("Bundle checksum/length mismatch"));
    }
    Ok(bytes)
}
/// Export into a NEW directory. On a normal failure remove only files created by this call.
pub fn export(path: &Path, tools: &Library, materials: &Materials) -> io::Result<()> {
    if !tools.valid() || !materials.valid() {
        return Err(bad("Invalid tools/materials"));
    }
    let tools = tools.portable();
    let mut assets = BTreeMap::new();
    for t in &tools.tools {
        for p in [&t.preset, &t.default] {
            for (id, kind) in refs(p) {
                let a = materials.get(id, kind).ok_or_else(|| {
                    bad(format!("Missing material {id}; portable export refused"))
                })?;
                assets.insert(id, a);
            }
        }
    }
    let mut payloads = vec![("tools.json".to_owned(), tools.encode()?)];
    for (id, a) in assets {
        payloads.push((
            format!("materials/{id}.json"),
            serde_json::to_vec(a).map_err(io::Error::other)?,
        ));
    }
    if payloads.iter().map(|(_, b)| b.len()).sum::<usize>() > LIMIT {
        return Err(bad("Bundle exceeds 12 MiB"));
    }
    let records = payloads
        .iter()
        .map(|(p, b)| File {
            path: p.clone(),
            bytes: b.len(),
            hash: hash(b),
        })
        .collect::<Vec<_>>();
    let mut iter = records.into_iter();
    let manifest = Manifest {
        version: 1,
        checksum: "fnv1a64-v1".into(),
        tools: iter.next().unwrap(),
        materials: iter.collect(),
    };
    payloads.push((
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?,
    ));
    fs::create_dir(path)?;
    let mut created: Vec<PathBuf> = vec![];
    let mut subdir = false;
    let result = (|| {
        fs::create_dir(path.join("materials"))?;
        subdir = true;
        for (name, bytes) in payloads {
            let dest = path.join(name);
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&dest)?;
            created.push(dest);
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        for f in created.iter().rev() {
            let _ = fs::remove_file(f);
        }
        if subdir {
            let _ = fs::remove_dir(path.join("materials"));
        }
        let _ = fs::remove_dir(path);
    }
    result
}
/// Validate every byte/reference/capacity first. No disk or caller state is changed.
pub fn import(
    path: &Path,
    current: &Library,
    materials: &Materials,
) -> io::Result<(Library, Materials)> {
    if !metadata(path)?.is_dir() {
        return Err(bad("Expected bundle directory"));
    }
    let root = path.canonicalize()?;
    let manifest: Manifest = serde_json::from_slice(&read(&root.join("manifest.json"), 65536)?)
        .map_err(io::Error::other)?;
    if manifest.version != 1
        || manifest.checksum != "fnv1a64-v1"
        || manifest.tools.path != "tools.json"
        || manifest.materials.len() > 32
    {
        return Err(bad("Unsupported bundle manifest"));
    }
    let mut seen = BTreeSet::from(["manifest.json".to_owned(), "tools.json".to_owned()]);
    let mut total = manifest.tools.bytes;
    for f in &manifest.materials {
        if asset_id(&f.path).is_none() || !seen.insert(f.path.clone()) {
            return Err(bad("Invalid/duplicate material path"));
        }
        total = total
            .checked_add(f.bytes)
            .ok_or_else(|| bad("Bundle size overflow"))?;
    }
    if total > LIMIT {
        return Err(bad("Bundle exceeds 12 MiB"));
    }
    if !metadata(&root.join("materials"))?.is_dir() {
        return Err(bad("Missing material directory"));
    }
    for dir in [&root, &root.join("materials")] {
        for item in fs::read_dir(dir)? {
            let p = item?.path();
            let relative = p
                .strip_prefix(&root)
                .map_err(io::Error::other)?
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "materials" {
                continue;
            }
            if !seen.contains(&relative) {
                return Err(bad("Unexpected bundle file"));
            }
            metadata(&p)?;
        }
    }
    let mut group = Library::decode(&checked(&root, &manifest.tools, 4 * 1024 * 1024)?)?;
    // No active engine cache or global defaults can smuggle unresolved dependencies.
    group = group.portable();
    let mut decoded = BTreeMap::new();
    for f in &manifest.materials {
        let a: Asset = serde_json::from_slice(&checked(&root, f, 2 * 1024 * 1024)?)
            .map_err(io::Error::other)?;
        if !a.valid() || Some(a.id) != asset_id(&f.path) {
            return Err(bad("Invalid material definition/ID"));
        }
        decoded.insert(a.id, a);
    }
    let mut used = BTreeSet::new();
    for t in &group.tools {
        for p in [&t.preset, &t.default] {
            for (id, kind) in refs(p) {
                if decoded.get(&id).is_none_or(|a| a.kind != kind) {
                    return Err(bad("Missing/wrong-kind bundled material"));
                }
                used.insert(id);
            }
        }
    }
    if used.len() != decoded.len() {
        return Err(bad("Unreferenced bundle material"));
    }
    let mut next_materials = materials.clone();
    let mut map = BTreeMap::new();
    for (old, a) in decoded {
        next_materials = next_materials.register_asset(a, true)?;
        map.insert(old, next_materials.assets().last().unwrap().id);
    }
    for t in &mut group.tools {
        remap(&mut t.preset, &map);
        remap(&mut t.default, &map);
    }
    let mut next = current.clone();
    next.import_group(&group).map_err(bad)?;
    Ok((next, next_materials))
}

/// Independent grey/alpha/fibre fixtures, portable ID remapping and local list history.
pub fn verify(root: &Path) -> Result<(Library, Materials), String> {
    use crate::{
        assets::BrushTip,
        fibres::{Fibre, Set},
        material_library::Density,
        tool_properties::Property,
        workspaces::Snapshot,
    };
    use image::ImageEncoder;
    let path = root.join("owned-density.png");
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[0, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 255],
            3,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    fs::write(&path, &png).map_err(|e| e.to_string())?;
    let mut mats = Materials::default()
        .import_density(&path, "Density", Kind::Tip, Density::LumaOrAlpha)
        .map_err(|e| e.to_string())?;
    if mats.assets()[0].values != [0, 128, 255]
        || fs::read(&path).map_err(|e| e.to_string())? != png
    {
        return Err("Direct grey density / source preservation".into());
    }
    let mut png_alpha = vec![];
    image::codecs::png::PngEncoder::new(&mut png_alpha)
        .write_image(
            &[200, 20, 50, 0, 200, 20, 50, 128, 200, 20, 50, 255],
            3,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    let alpha = root.join("owned-alpha.png");
    fs::write(&alpha, &png_alpha).map_err(|e| e.to_string())?;
    mats = mats
        .import_density(&alpha, "Alpha", Kind::Tip, Density::LumaOrAlpha)
        .map_err(|e| e.to_string())?;
    if mats.assets()[1].values != [0, 128, 255] {
        return Err("Alpha density differs from input".into());
    }
    let many = Set {
        version: 1,
        fibres: vec![
            Fibre {
                position: [-0.3, 0.0],
                ..Default::default()
            },
            Fibre {
                position: [0.3, 0.0],
                angle: 90.0,
                density: 0.5,
                ..Default::default()
            },
        ],
    };
    mats = mats
        .register_fibres("Fibres", many)
        .map_err(|e| e.to_string())?;
    mats = mats
        .register_fibres("Empty fibres", Set::default())
        .map_err(|e| e.to_string())?;
    let s = Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    let mut tools = Library::default();
    for asset in mats.assets() {
        let mut p = Preset::capture(&s, BrushTip::Pen, &asset.name)?;
        p.sequence.enabled = true;
        p.sequence.tips[0] = asset.id;
        p.values[0] = 8.0;
        p.pressure.size = false;
        p.pressure.opacity = false;
        tools.create_tool(1, p, Property::ALL.to_vec())?;
    }
    let ids = tools.tools.iter().map(|t| t.id).collect::<Vec<_>>();
    let original = tools.clone();
    let mut history = crate::user_tools::History::default();
    tools.drop_before(ids[2], ids[0], false)?;
    history.record(original.clone(), &tools);
    if tools.tools[0].id != ids[2] {
        return Err("Slot move did not preserve stable ID".into());
    }
    let moved = tools.clone();
    let copy = tools.drop_before(ids[0], ids[1], true)?;
    history.record(moved.clone(), &tools);
    tools
        .tools
        .iter_mut()
        .find(|t| t.id == copy)
        .unwrap()
        .preset
        .values[0] = 19.0;
    if tools.entry(ids[0]).unwrap().preset.values[0] != 8.0 {
        return Err("Slot copy shares mutable settings".into());
    }
    if !history.undo(&mut tools)
        || tools != moved
        || !history.undo(&mut tools)
        || tools != original
        || !history.redo(&mut tools)
        || tools != moved
    {
        return Err("Tool-list undo/redo failed".into());
    }
    let bundle = root.join("owned-tool-bundle");
    export(&bundle, &tools, &mats).map_err(|e| e.to_string())?;
    let manifest_before = fs::read(bundle.join("manifest.json")).map_err(|e| e.to_string())?;
    if export(&bundle, &tools, &mats).is_ok()
        || fs::read(bundle.join("manifest.json")).map_err(|e| e.to_string())? != manifest_before
    {
        return Err("Export replaced existing directory".into());
    }
    // Simulated second offline PC: a colliding ID and name are already occupied.
    let existing = Materials::default()
        .register_fibres(
            "Density",
            Set {
                version: 1,
                fibres: vec![Fibre::default()],
            },
        )
        .map_err(|e| e.to_string())?;
    let (new_tools, new_mats) =
        import(&bundle, &Library::default(), &existing).map_err(|e| e.to_string())?;
    if new_mats.assets().len() != 5
        || new_mats.assets()[0] != existing.assets()[0]
        || new_tools
            .tools
            .iter()
            .map(|t| &t.preset.name)
            .collect::<Vec<_>>()
            != tools
                .tools
                .iter()
                .map(|t| &t.preset.name)
                .collect::<Vec<_>>()
    {
        return Err("Portable import lost order / existing material".into());
    }
    for (old, new) in tools.tools.iter().zip(&new_tools.tools) {
        let a = mats
            .get(old.preset.sequence.tips[0], Kind::Tip)
            .ok_or("Old dependency")?;
        let b = new_mats
            .get(new.preset.sequence.tips[0], Kind::Tip)
            .ok_or("New dependency")?;
        if a.id == b.id || a.values != b.values || a.fibres != b.fibres {
            return Err("Portable dependency ID remapping failed".into());
        }
        for y in 0..16 {
            for x in 0..16 {
                let q = [x as f64 / 8.0 - 0.9375, y as f64 / 8.0 - 0.9375];
                if a.tip(q) != b.tip(q) {
                    return Err("Offline restored material changed coverage".into());
                }
            }
        }
    }
    let tools_path = root.join("portable-slots.json");
    new_tools
        .save(&tools_path, false)
        .map_err(|e| e.to_string())?;
    let mats_path = root.join("portable-materials.json");
    new_mats.save(&mats_path).map_err(|e| e.to_string())?;
    if Library::read(&tools_path).map_err(|e| e.to_string())? != new_tools
        || Materials::read(&mats_path).map_err(|e| e.to_string())? != new_mats
    {
        return Err("Slots/materials did not survive restart".into());
    }
    Ok((new_tools, new_mats))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_owned_materials_and_slot_history() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn corrupt_missing_path_duplicate_size_and_version_preserve_everything() {
        let root = crate::test_support::TempDir::new();
        let (current, mats) = verify(&root.0).unwrap();
        let bundle = root.0.join("owned-tool-bundle");
        let manifest_path = bundle.join("manifest.json");
        let original = fs::read(&manifest_path).unwrap();
        let before = current.encode().unwrap();
        let masks = serde_json::to_vec(&mats).unwrap();
        for mutate in 0..7 {
            let mut m: Manifest = serde_json::from_slice(&original).unwrap();
            match mutate {
                0 => m.version = 2,
                1 => m.materials[0].path = "../escape.json".into(),
                2 => m.materials[0].path = "C:/escape.json".into(),
                3 => m.materials[0].hash = "0000000000000000".into(),
                4 => m.materials[0].bytes = usize::MAX,
                5 => m.materials[1].path = m.materials[0].path.clone(),
                _ => m.tools.path = "materials/../tools.json".into(),
            }
            fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).unwrap();
            assert!(import(&bundle, &current, &mats).is_err());
            assert_eq!(current.encode().unwrap(), before);
            assert_eq!(serde_json::to_vec(&mats).unwrap(), masks);
        }
        fs::write(&manifest_path, &original).unwrap();
        let m: Manifest = serde_json::from_slice(&original).unwrap();
        let f = bundle.join(&m.materials[0].path);
        let bytes = fs::read(&f).unwrap();
        fs::remove_file(&f).unwrap();
        assert!(import(&bundle, &current, &mats).is_err());
        fs::write(&f, &bytes).unwrap();
        fs::write(bundle.join("unlisted.json"), b"{}").unwrap();
        assert!(import(&bundle, &current, &mats).is_err());
        fs::remove_file(bundle.join("unlisted.json")).unwrap();
        // Correct checksum cannot make a semantically invalid material acceptable.
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["width"] = serde_json::json!(0);
        let changed = serde_json::to_vec(&value).unwrap();
        fs::write(&f, &changed).unwrap();
        let mut m: Manifest = serde_json::from_slice(&original).unwrap();
        m.materials[0].bytes = changed.len();
        m.materials[0].hash = hash(&changed);
        fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(import(&bundle, &current, &mats).is_err());
    }
}
