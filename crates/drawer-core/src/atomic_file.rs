//! Verified temporary writes, a retained healthy predecessor and one rename commit.
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Created,
    Written,
    Flushed,
    Synced,
    Verified,
    BackupReady,
    Replacing,
    Published,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub preserved_unverified: Option<PathBuf>,
}
struct Owned(Vec<PathBuf>);
impl Drop for Owned {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = fs::remove_file(p);
        }
    }
}
pub fn predecessor(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".drawer-last-good");
    path.with_file_name(name)
}
fn stamp(path: &Path) -> io::Result<Option<(u64, [u8; 32])>> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !m.is_file() || crate::asset_catalog::linked(&m) || m.len() > 2 * 1024 * 1024 * 1024 {
        return Err(io::Error::other(
            "Save destination must be a regular local file within2GiB",
        ));
    }
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    let mut count = 0u64;
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other("Save size overflow"))?;
        if count > m.len() {
            return Err(io::Error::other("Save destination changed while reading"));
        }
        h.update(&b[..n]);
    }
    if count != m.len() {
        return Err(io::Error::other("Save destination changed while reading"));
    }
    Ok(Some((count, h.finalize().into())))
}
pub(crate) fn publish(
    path: &Path,
    temporary: &Path,
    write: impl FnOnce(&mut BufWriter<fs::File>) -> io::Result<()>,
    validate: impl Fn(&Path) -> io::Result<()>,
    tick: &mut impl FnMut(Stage) -> io::Result<()>,
) -> io::Result<Outcome> {
    let original = stamp(path)?;
    let healthy = original.is_some() && validate(path).is_ok();
    let mut outcome = Outcome {
        preserved_unverified: None,
    };
    let backup = predecessor(path);
    let mut backup_name = backup.file_name().unwrap().to_owned();
    backup_name.push("-write.tmp");
    let backup_temp = backup.with_file_name(backup_name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    let mut owned = Owned(vec![temporary.to_path_buf()]);
    let mut writer = BufWriter::new(file);
    tick(Stage::Created)?;
    write(&mut writer)?;
    tick(Stage::Written)?;
    writer.flush()?;
    tick(Stage::Flushed)?;
    writer.get_ref().sync_all()?;
    tick(Stage::Synced)?;
    drop(writer);
    validate(temporary)?;
    tick(Stage::Verified)?;
    if original.is_some() && healthy {
        // Never follow or replace an unreviewed link at the predecessor path.
        if stamp(&backup)?.is_some() {
            validate(&backup)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup_temp)?;
        owned.0.push(backup_temp.clone());
        let mut input = fs::File::open(path)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        drop(input);
        validate(&backup_temp)?;
        if stamp(&backup_temp)? != original || stamp(path)? != original {
            return Err(io::Error::other(
                "Destination changed; current file preserved",
            ));
        }
        fs::rename(&backup_temp, &backup)?;
        owned.0.retain(|p| p != &backup_temp);
    }
    if let Some((_, hash)) = &original
        && !healthy
    {
        let hash = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut name = path.file_name().unwrap().to_owned();
        name.push(format!(".drawer-unverified-{hash}"));
        let copy = path.with_file_name(name);
        match stamp(&copy)? {
            Some(_) if stamp(&copy)? != original => {
                return Err(io::Error::other(
                    "Unverified predecessor name collision; existing files preserved",
                ));
            }
            Some(_) => {}
            None => {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&copy)?;
                owned.0.push(copy.clone());
                let mut input = fs::File::open(path)?;
                io::copy(&mut input, &mut output)?;
                output.sync_all()?;
                drop(output);
                drop(input);
                if stamp(&copy)? != original || stamp(path)? != original {
                    return Err(io::Error::other(
                        "Original changed while preserving unreadable bytes",
                    ));
                }
                owned.0.retain(|p| p != &copy);
            }
        }
        outcome.preserved_unverified = Some(copy);
    }
    tick(Stage::BackupReady)?;
    tick(Stage::Replacing)?;
    if stamp(path)? != original {
        return Err(io::Error::other(
            "Destination changed before replacement; current file preserved",
        ));
    }
    fs::rename(temporary, path)?;
    owned.0.retain(|p| p != temporary);
    let _ = tick(Stage::Published); // Publication already happened; a progress callback cannot roll it back.
    Ok(outcome)
}

#[path = "atomic_file_verify.rs"]
pub mod verify;
