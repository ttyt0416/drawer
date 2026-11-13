//! Persisted device policy for the packaged CPU-only engine; no SDK discovery/download.
use drawer_core::storage::UserStorage;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Device {
    #[default]
    Cpu,
    Gpu,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    version: u32,
    pub device: Device,
    pub allow_cpu_fallback: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            device: Device::Cpu,
            allow_cpu_fallback: false,
        }
    }
}
impl Settings {
    pub fn execution(self) -> Result<&'static str, String> {
        match (self.device,self.allow_cpu_fallback) {
            (Device::Cpu,_)=>Ok("Execution device: CPU (bundled ONNX engine; no GPU SDK required)"),
            (Device::Gpu,true)=>Ok("Execution device: CPU fallback (GPU requested; fallback explicitly enabled)"),
            (Device::Gpu,false)=>Err("GPU unavailable: this local build includes no GPU execution adapter/SDK. Choose CPU or explicitly enable CPU fallback. No SDK/model will be downloaded; basic editing and saving remain available.".into()),
        }
    }
    fn read(path: &Path) -> io::Result<Self> {
        let path = drawer_core::file_browser::checked(path).map_err(io::Error::other)?;
        let mut data = Vec::new();
        std::fs::File::open(path)?
            .take(4097)
            .read_to_end(&mut data)?;
        if data.len() > 4096 {
            return Err(io::Error::other("Local compute settings limit4KiB"));
        }
        let s: Self = serde_json::from_slice(&data).map_err(io::Error::other)?;
        if s.version != 1 {
            return Err(io::Error::other(
                "Unsupported local compute settings version; original preserved",
            ));
        }
        Ok(s)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        let path = drawer_core::file_browser::new_path(
            &storage.root().join("settings/local-compute.json"),
        )
        .map_err(io::Error::other)?;
        if !path.try_exists()? {
            return Ok(Self::default());
        }
        Self::read(&path)
    }
    pub fn save(self, storage: &UserStorage) -> io::Result<()> {
        if self.version != 1 {
            return Err(io::Error::other("Invalid compute settings version"));
        }
        let path = drawer_core::file_browser::new_path(
            &storage.root().join("settings/local-compute.json"),
        )
        .map_err(io::Error::other)?;
        let temp =
            drawer_core::file_browser::new_path(&path.with_file_name("local-compute-write.tmp"))
                .map_err(io::Error::other)?;
        if drawer_platform::file_browser::available_space(storage.root())
            .map_err(io::Error::other)?
            < 128 * 1024
        {
            return Err(io::Error::other(
                "Insufficient local disk space for compute settings",
            ));
        }
        let data = serde_json::to_vec(&self).map_err(io::Error::other)?;
        drawer_core::atomic_file::publish(
            &path,
            &temp,
            |w| w.write_all(&data),
            |p| Self::read(p).map(|_| ()),
            &mut |_| Ok(()),
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_default_explicit_fallback_restart_and_failed_write_preserve_policy() {
        let root = crate::tests::TestDirectory::new();
        let storage = UserStorage::at(root.0.join("한글 compute")).unwrap();
        assert_eq!(Settings::load(&storage).unwrap(), Settings::default());
        let gpu = Settings {
            device: Device::Gpu,
            ..Default::default()
        };
        assert!(gpu.execution().is_err());
        gpu.save(&storage).unwrap();
        assert_eq!(Settings::load(&storage).unwrap(), gpu);
        let cpu = Settings {
            allow_cpu_fallback: true,
            ..gpu
        };
        assert!(cpu.execution().unwrap().contains("CPU fallback"));
        cpu.save(&storage).unwrap();
        assert_eq!(Settings::load(&storage).unwrap(), cpu);
        let path = storage.root().join("settings/local-compute.json");
        let before = std::fs::read(&path).unwrap();
        let foreign = storage.root().join("settings/local-compute-write.tmp");
        std::fs::write(&foreign, b"foreign interrupted bytes").unwrap();
        assert!(gpu.save(&storage).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"foreign interrupted bytes"
        );
        std::fs::write(
            &path,
            br#"{"version":999,"device":"Cpu","allow_cpu_fallback":false}"#,
        )
        .unwrap();
        let corrupt = std::fs::read(&path).unwrap();
        assert!(Settings::load(&storage).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    }
}
