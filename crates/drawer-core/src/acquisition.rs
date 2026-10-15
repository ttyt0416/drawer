//! Bounded local-device helper protocol. No device access occurs in this crate.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    WiaScanner,
    WiaCamera,
    Camera,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub kind: Kind,
    pub id: String,
    pub name: String,
}
impl Device {
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 4096
            && !self.id.contains('\0')
            && !self.name.is_empty()
            && self.name.len() <= 1024
            && !self.name.contains('\0')
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    List,
    Capture(Device),
    /// Protocol-only diagnostic; never opens/enumerates a device.
    Probe(u32),
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Reply {
    Devices {
        devices: Vec<Device>,
        warnings: Vec<String>,
    },
    Image(PathBuf),
    Failure(String),
}
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
pub fn read<T: DeserializeOwned>(path: &Path, limit: u64) -> io::Result<T> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(invalid("Device protocol exceeds its size limit"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("Device protocol grew past its limit"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid device protocol"))
}
pub fn request(path: &Path) -> io::Result<Request> {
    let r = read(path, 16384)?;
    if matches!(&r,Request::Capture(d) if !d.valid()) {
        return Err(invalid("Invalid selected local device"));
    }
    if matches!(r,Request::Probe(ms) if ms>2000) {
        return Err(invalid("Device protocol probe delay exceeds 2 seconds"));
    }
    Ok(r)
}
pub fn write<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    crate::workspaces::write_json(path, value)
}
pub fn reply(path: &Path) -> io::Result<Reply> {
    let r = read(path, 512 * 1024)?;
    match &r {
        Reply::Devices { devices, warnings }
            if devices.len() > 64
                || devices.iter().any(|d| !d.valid())
                || warnings.len() > 8
                || warnings.iter().any(|s| s.len() > 4096) =>
        {
            return Err(invalid("Invalid local device list"));
        }
        Reply::Failure(s) if s.len() > 16384 => {
            return Err(invalid("Invalid device failure response"));
        }
        _ => {}
    }
    Ok(r)
}
/// Only read the helper's own output. Never read/delete a driver-returned foreign
/// path, directory, symlink, request file or response file as an acquired image.
pub fn image_path(directory: &Path, path: &Path) -> io::Result<PathBuf> {
    let root = directory.canonicalize()?;
    let meta = fs::symlink_metadata(path)?;
    let absolute = path.canonicalize()?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || absolute.parent() != Some(root.as_path())
        || meta.len() > MAX_BYTES
        || meta.len() == 0
        || matches!(
            absolute.file_name().and_then(|v| v.to_str()),
            Some("request.json" | "response.json")
        )
    {
        return Err(invalid("Device returned an unsafe or oversized image path"));
    }
    Ok(absolute)
}
pub fn original(path: &Path) -> io::Result<Vec<u8>> {
    let f = fs::File::open(path)?;
    if f.metadata()?.len() > MAX_BYTES {
        return Err(invalid("Acquired image exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    f.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("Acquired image exceeds its size limit"));
    }
    Ok(bytes)
}
pub fn save_original(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("Invalid acquired original"));
    }
    // Create-new is deliberate: an original never overwrites an existing file.
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    #[test]
    fn protocol_limits_devices_unicode_and_damaged_responses() {
        let dir = TempDir::new();
        let request_file = dir.0.join("request.json");
        let response_file = dir.0.join("response.json");
        let device = Device {
            kind: Kind::Camera,
            id: "로컬-카메라".into(),
            name: "카메라".into(),
        };
        write(&request_file, &Request::Capture(device.clone())).unwrap();
        assert!(matches!(request(&request_file).unwrap(),Request::Capture(d) if d==device));
        write(&request_file, &Request::Probe(2001)).unwrap();
        assert!(request(&request_file).is_err());
        write(
            &response_file,
            &Reply::Devices {
                devices: vec![],
                warnings: vec![],
            },
        )
        .unwrap();
        assert!(
            matches!(reply(&response_file).unwrap(),Reply::Devices{devices,..} if devices.is_empty())
        );
        write(
            &response_file,
            &Reply::Devices {
                devices: vec![device; 65],
                warnings: vec![],
            },
        )
        .unwrap();
        assert!(reply(&response_file).is_err());
        fs::write(&response_file, b"{invalid").unwrap();
        assert!(reply(&response_file).is_err());
        let file = fs::File::create(&response_file).unwrap();
        file.set_len(512 * 1024 + 1).unwrap();
        drop(file);
        assert!(reply(&response_file).is_err());
    }
    #[test]
    fn foreign_paths_oversized_images_and_existing_original_are_preserved() {
        let dir = TempDir::new();
        let work = dir.0.join("work");
        fs::create_dir(&work).unwrap();
        let foreign = dir.0.join("user-picture.png");
        fs::write(&foreign, b"user original").unwrap();
        assert!(image_path(&work, &foreign).is_err());
        assert!(save_original(&foreign, b"replacement").is_err());
        assert_eq!(fs::read(&foreign).unwrap(), b"user original");
        let own = work.join("capture.bmp");
        save_original(&own, b"own image").unwrap();
        assert_eq!(
            original(&image_path(&work, &own).unwrap()).unwrap(),
            b"own image"
        );
        let file = fs::File::create(&own).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        drop(file);
        assert!(image_path(&work, &own).is_err());
        assert!(original(&own).is_err());
        fs::write(work.join("request.json"), b"request").unwrap();
        assert!(image_path(&work, &work.join("request.json")).is_err());
    }
}
