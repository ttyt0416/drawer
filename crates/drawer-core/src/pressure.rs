use serde::{Deserialize, Serialize};
use std::{io, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub size: bool,
    pub opacity: bool,
}
impl Default for Mapping {
    fn default() -> Self {
        Self {
            size: true,
            opacity: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub tools: [Mapping; 4],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            tools: [Mapping::default(); 4],
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if std::fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Pressure preset exceeds 64 KiB"));
        }
        let s: Self = serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        if !s.valid() {
            return Err(io::Error::other("Invalid pressure preset"));
        }
        Ok(s)
    }
    pub fn save(self, path: &Path) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid pressure preset"));
        }
        crate::workspaces::write_json(path, &self)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_workspace_defaults_and_import_scope_preserve_pressure() {
        use crate::workspaces::{Scope, Snapshot};
        let mut value = serde_json::to_value(Snapshot::default()).unwrap();
        value.as_object_mut().unwrap().remove("pressure");
        let legacy: Snapshot = serde_json::from_value(value).unwrap();
        assert!(legacy.valid());
        assert_eq!(legacy.pressure, Settings::default());
        let mut incoming = legacy.clone();
        incoming.pressure.tools[2].opacity = true;
        let (layout, _) = legacy
            .merge(
                &incoming,
                Scope {
                    layout: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(layout.pressure, legacy.pressure);
        let (other, _) = legacy
            .merge(
                &incoming,
                Scope {
                    other: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(other.pressure, incoming.pressure);
    }
}
