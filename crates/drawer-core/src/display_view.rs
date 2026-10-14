use crate::{storage::UserStorage, view::View};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub key: String,
    pub index: usize,
    pub position: [i32; 2],
    pub size: [u32; 2],
    pub scale: f64,
    pub primary: bool,
}
impl Monitor {
    pub fn valid(&self) -> bool {
        !self.key.is_empty()
            && self.key.len() <= 512
            && self.size.iter().all(|n| (1..=65536).contains(n))
            && self.scale.is_finite()
            && (0.25..=8.0).contains(&self.scale)
    }
    /// Window placement in physical desktop pixels; convert with the receiving viewport's scale.
    pub fn window(&self) -> ([f64; 2], [f64; 2]) {
        let margin = (self.size[0].min(self.size[1]) as f64 * 0.05).min(48.0);
        let size = [
            (self.size[0] as f64 - margin * 2.0).clamp(1.0, 1024.0),
            (self.size[1] as f64 - margin * 2.0).clamp(1.0, 720.0),
        ];
        (
            [
                self.position[0] as f64 + margin,
                self.position[1] as f64 + margin,
            ],
            size,
        )
    }
}
pub fn selected<'a>(monitors: &'a [Monitor], preferred: Option<&str>) -> Option<&'a Monitor> {
    monitors
        .iter()
        .find(|m| m.valid() && Some(m.key.as_str()) == preferred)
        .or_else(|| monitors.iter().find(|m| m.valid() && m.primary))
        .or_else(|| monitors.iter().find(|m| m.valid()))
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Framing {
    pub zoom: f64,
    pub angle: f64,
    pub flipped: bool,
    pub center: [f64; 2],
}
impl Default for Framing {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            angle: 0.0,
            flipped: false,
            center: [0.5; 2],
        }
    }
}
impl Framing {
    pub fn valid(self) -> bool {
        self.zoom.is_finite()
            && (0.01..=64.0).contains(&self.zoom)
            && self.angle.is_finite()
            && self.center.iter().all(|n| n.is_finite() && n.abs() <= 1e6)
    }
    pub fn capture(view: View, size: [f64; 2], ppp: f64) -> Self {
        let center = view.screen_to_document([0.0; 2], size, [0.0; 2], ppp);
        Self {
            zoom: view.zoom,
            angle: view.angle,
            flipped: view.flipped,
            center: [center[0] / size[0], center[1] / size[1]],
        }
    }
    pub fn view(self, size: [f64; 2], ppp: f64) -> View {
        let mut view = View {
            zoom: self.zoom,
            angle: self.angle,
            flipped: self.flipped,
            pan: [0.0; 2],
        };
        view.center_on(
            [self.center[0] * size[0], self.center[1] * size[1]],
            size,
            [0.0; 2],
            ppp,
        );
        view
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub visible: bool,
    pub preferred: Option<String>,
    pub synced: bool,
    pub fullscreen: bool,
    pub framing: Framing,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            visible: false,
            preferred: None,
            synced: true,
            fullscreen: true,
            framing: Default::default(),
        }
    }
}
fn read(path: &Path) -> io::Result<Preferences> {
    if fs::metadata(path)?.len() > 65536 {
        return Err(io::Error::other("Display preferences are too large"));
    }
    let prefs: Preferences = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
    if !prefs.valid() {
        return Err(io::Error::other("Invalid display preferences"));
    }
    Ok(prefs)
}
fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("json.write.tmp");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
impl Preferences {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.framing.valid()
            && self
                .preferred
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 512 && !s.contains('\0'))
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match read(&storage.root().join("settings/display.json")) {
            Ok(prefs) => Ok(prefs),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn previous(storage: &UserStorage) -> io::Result<Self> {
        read(&storage.root().join("settings/display.previous.json"))
    }
    pub fn save(&self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid display preferences"));
        }
        let path = storage.root().join("settings/display.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(io::Error::other(
                    "Display preferences exceed the recovery limit; original preserved",
                ));
            }
            let name = if read(&path).is_ok() {
                "display.previous.json"
            } else {
                "display.damaged.json"
            };
            write(
                &storage.root().join("settings").join(name),
                &fs::read(&path)?,
            )?;
        }
        write(&path, &serde_json::to_vec(self).map_err(io::Error::other)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    #[test]
    fn mixed_scale_negative_coordinates_disconnect_reconnect_and_window_bounds() {
        let a = Monitor {
            key: "main".into(),
            index: 0,
            position: [0, 0],
            size: [1920, 1080],
            scale: 1.0,
            primary: true,
        };
        let b = Monitor {
            key: "external".into(),
            index: 1,
            position: [-3840, -200],
            size: [3840, 2160],
            scale: 2.0,
            primary: false,
        };
        let both = [a.clone(), b.clone()];
        assert_eq!(selected(&both, Some("external")), Some(&b));
        assert_eq!(
            selected(std::slice::from_ref(&a), Some("external")),
            Some(&a)
        );
        assert_eq!(selected(&both, Some("external")), Some(&b));
        assert!(selected(&[], None).is_none());
        for monitor in both {
            let (position, size) = monitor.window();
            for i in 0..2 {
                assert!(position[i] >= monitor.position[i] as f64);
                assert!(
                    position[i] + size[i] <= monitor.position[i] as f64 + monitor.size[i] as f64
                );
                assert!(((position[i] / monitor.scale) * monitor.scale - position[i]).abs() < 1e-9);
            }
        }
    }
    #[test]
    fn framing_keeps_device_zoom_and_document_center_across_dpi_and_resolution() {
        let size = [1200.0, 900.0];
        for ppp in [1.0, 1.5, 2.0] {
            for target in [1.0, 1.5, 2.0] {
                let original = View {
                    zoom: 2.0,
                    angle: 37.0,
                    flipped: true,
                    pan: [120.0, -75.0],
                };
                let framing = Framing::capture(original, size, ppp);
                let transferred = framing.view(size, target);
                let a = original.screen_to_document([0.0; 2], size, [0.0; 2], ppp);
                let b = transferred.screen_to_document([0.0; 2], size, [0.0; 2], target);
                for i in 0..2 {
                    assert!((a[i] - b[i]).abs() < 1e-8);
                }
                assert_eq!(transferred.zoom, original.zoom);
                assert_eq!(transferred.angle, original.angle);
                assert_eq!(transferred.flipped, original.flipped);
            }
        }
    }
    #[test]
    fn local_preferences_backup_corruption_and_failed_save_preserve_normal_settings() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let first = Preferences {
            preferred: Some("external".into()),
            visible: true,
            ..Default::default()
        };
        first.save(&storage).unwrap();
        assert_eq!(Preferences::load(&storage).unwrap(), first);
        let second = Preferences {
            fullscreen: false,
            synced: false,
            ..first.clone()
        };
        second.save(&storage).unwrap();
        assert_eq!(Preferences::previous(&storage).unwrap(), first);
        fs::write(
            storage.root().join("settings/display.json.write.tmp"),
            "foreign",
        )
        .unwrap();
        assert!(first.save(&storage).is_err());
        assert_eq!(Preferences::load(&storage).unwrap(), second);
        fs::remove_file(storage.root().join("settings/display.json.write.tmp")).unwrap();
        fs::write(storage.root().join("settings/display.json"), "damaged").unwrap();
        assert!(Preferences::load(&storage).is_err());
        first.save(&storage).unwrap();
        assert_eq!(
            fs::read_to_string(storage.root().join("settings/display.damaged.json")).unwrap(),
            "damaged"
        );
        assert!(
            !Preferences {
                framing: Framing {
                    zoom: f64::NAN,
                    ..Default::default()
                },
                ..Default::default()
            }
            .valid()
        );
    }
}
