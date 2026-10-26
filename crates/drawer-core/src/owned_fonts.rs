//! Only manifest-owned copies under the application's font directory may be removed.
use super::*;
use crate::storage::UserStorage;
use std::{
    fs,
    io::{self, Read, Write},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owned {
    pub digest: [u8; 32],
    pub file: String,
    pub faces: Vec<Reference>,
}
fn err(s: &str) -> io::Error {
    io::Error::other(s)
}
fn linked(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return true;
            }
        }
        m.file_type().is_symlink()
    })
}
fn root(storage: &UserStorage) -> io::Result<PathBuf> {
    let base = storage.root().canonicalize()?;
    if !local(&base) {
        return Err(err("Choose local app storage"));
    }
    let path = base.join("fonts");
    if linked(&path) {
        return Err(err("The app font folder cannot be a link"));
    }
    fs::create_dir_all(&path)?;
    let path = path.canonicalize()?;
    if path.parent() != Some(base.as_path()) {
        return Err(err("Font folder escaped app storage"));
    }
    Ok(path)
}
fn hex(d: [u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}
fn valid(e: &Owned) -> bool {
    let stem = format!("drawer-{}", hex(e.digest));
    ["ttf", "otf", "ttc", "otc"]
        .iter()
        .any(|ext| e.file == format!("{stem}.{ext}"))
        && !e.faces.is_empty()
        && e.faces.len() <= 64
        && e.faces.iter().all(|r| r.valid())
}
pub fn load(storage: &UserStorage) -> io::Result<Vec<Owned>> {
    let path = root(storage)?.join("drawer-fonts.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    if linked(&path) {
        return Err(err("Font manifest cannot be a link"));
    }
    let mut data = vec![];
    fs::File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    if data.len() > 4 * 1024 * 1024 {
        return Err(err("Font manifest limit"));
    }
    let entries: Vec<Owned> = serde_json::from_slice(&data).map_err(io::Error::other)?;
    if entries.len() > 128
        || entries.iter().map(|e| e.faces.len()).sum::<usize>() > 1024
        || entries
            .iter()
            .enumerate()
            .any(|(i, e)| !valid(e) || entries[..i].iter().any(|p| p.file == e.file))
    {
        return Err(err("Invalid font ownership manifest"));
    }
    Ok(entries)
}
fn save(root: &Path, entries: &[Owned]) -> io::Result<()> {
    let temp = root.join("drawer-fonts-write.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        f.write_all(&serde_json::to_vec(entries).map_err(io::Error::other)?)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temp, root.join("drawer-fonts.json"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn lock(root: &Path) -> io::Result<Lock> {
    let path = root.join("drawer-font-management.lock");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok(Lock(path))
}
fn checked(root: &Path, e: &Owned) -> io::Result<PathBuf> {
    if !valid(e) {
        return Err(err("Invalid owned font"));
    }
    let path = root.join(&e.file);
    if linked(&path)
        || !fs::metadata(&path)?.is_file()
        || path.canonicalize()?.parent() != Some(root)
    {
        return Err(err("Only regular app-owned font files can be changed"));
    }
    if <[u8; 32]>::from(Sha256::digest(read_font(&path)?)) != e.digest {
        return Err(err("App font changed outside Drawer; file preserved"));
    }
    Ok(path)
}
pub fn install(storage: &UserStorage, source: &Path, licensed: bool) -> io::Result<Owned> {
    if !licensed {
        return Err(err("Confirm permission to copy this font for local use"));
    }
    if linked(source) {
        return Err(err("Choose a regular font source"));
    }
    let source = source.canonicalize()?;
    if !local(&source) || !fs::metadata(&source)?.is_file() {
        return Err(err("Choose a local regular font"));
    }
    let ext = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["ttf", "otf", "ttc", "otc"].contains(&ext.as_str()) {
        return Err(err("Choose TTF/OTF/TTC/OTC"));
    }
    let bytes = read_font(&source)?;
    let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
    if !(1..=64).contains(&count) {
        return Err(err("Font collection face limit64"));
    }
    let hash = prefix(&bytes);
    let mut faces = vec![];
    for index in 0..count {
        let face = ttf_parser::Face::parse(&bytes, index).map_err(io::Error::other)?;
        let name = face
            .names()
            .into_iter()
            .filter(|n| {
                matches!(
                    n.name_id,
                    ttf_parser::name_id::FULL_NAME | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                )
            })
            .find_map(|n| n.to_string())
            .unwrap_or_else(|| {
                source
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        faces.push(Reference::new(identity(&hash, index), index, &name));
    }
    let root = root(storage)?;
    let _lock = lock(&root)?;
    let mut entries = load(storage)?;
    let digest = Sha256::digest(&bytes).into();
    if let Some(old) = entries.iter().find(|e| e.digest == digest) {
        checked(&root, old)?;
        return Ok(old.clone());
    }
    if entries.len() >= 128
        || entries.iter().map(|e| e.faces.len()).sum::<usize>() + faces.len() > 1024
    {
        return Err(err("App font limit128 files/1024 faces"));
    }
    let e = Owned {
        digest,
        file: format!("drawer-{}.{}", hex(digest), ext),
        faces,
    };
    let path = root.join(&e.file);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        entries.push(e.clone());
        save(&root, &entries)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    for face in &e.faces {
        register(*face, &path);
    }
    invalidate_data();
    Ok(e)
}
pub fn remove(storage: &UserStorage, digest: [u8; 32], confirmation: &str) -> io::Result<()> {
    let root = root(storage)?;
    let _lock = lock(&root)?;
    let mut entries = load(storage)?;
    let index = entries
        .iter()
        .position(|e| e.digest == digest)
        .ok_or_else(|| err("This is not an app-owned font"))?;
    let entry = &entries[index];
    if confirmation != entry.faces[0].name() {
        return Err(err(
            "Confirm the displayed font name; the whole app-owned collection will be removed",
        ));
    }
    let path = checked(&root, entry)?;
    let quarantine = root.join(format!("{}.delete.tmp", entry.file));
    if quarantine.exists() {
        return Err(err("Font removal temporary already exists; preserved"));
    }
    let previous = entries.clone();
    fs::rename(&path, &quarantine)?;
    entries.remove(index);
    if let Err(error) = save(&root, &entries) {
        fs::rename(&quarantine, &path)?;
        return Err(error);
    }
    if let Err(error) = fs::remove_file(&quarantine) {
        fs::rename(&quarantine, &path)?;
        save(&root, &previous)?;
        return Err(error);
    }
    invalidate_data();
    Ok(())
}
