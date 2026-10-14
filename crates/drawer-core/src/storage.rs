use crate::assets::BrushTip;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub struct UserStorage {
    root: PathBuf,
}

impl UserStorage {
    pub fn from_environment() -> io::Result<Self> {
        let root = match env::var_os("DRAWER_DATA_DIR") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(env::var_os("LOCALAPPDATA").ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable")
            })?)
            .join("Drawer"),
        };
        Self::at(root)
    }
    pub fn at(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(root.join("settings"))?;
        fs::create_dir_all(root.join("materials"))?;
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn load_view(&self) -> io::Result<crate::view::View> {
        let text = match fs::read_to_string(self.root.join("settings/view.txt")) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(crate::view::View::default());
            }
            Err(error) => return Err(error),
        };
        crate::view::View::decode(&text)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Damaged view preferences"))
    }
    pub fn save_view(&self, view: crate::view::View) -> io::Result<()> {
        let temporary = self.root.join("settings/view.tmp");
        let path = self.root.join("settings/view.txt");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(view.encode().as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    pub fn load_brush(&self) -> io::Result<BrushTip> {
        let path = self.root.join("settings/preferences.txt");
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BrushTip::Pen),
            Err(error) => return Err(error),
        };
        let name = text
            .strip_prefix("version=1\nbrush=")
            .and_then(|value| value.strip_suffix('\n'));
        BrushTip::ALL
            .into_iter()
            .find(|tip| Some(tip.name()) == name)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unsupported or damaged preferences",
                )
            })
    }
    pub fn save_brush(&self, brush: BrushTip) -> io::Result<()> {
        let path = self.root.join("settings/preferences.txt");
        let temporary = self.root.join("settings/preferences.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            write!(file, "version=1\nbrush={}\n", brush.name())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_and_failed_save_preserves_previous() {
        let root = env::temp_dir().join(format!("drawer-storage-{}", std::process::id()));
        let storage = UserStorage::at(root.clone()).unwrap();
        assert_eq!(storage.load_brush().unwrap(), BrushTip::Pen);
        storage.save_brush(BrushTip::Wash).unwrap();
        assert_eq!(storage.load_brush().unwrap(), BrushTip::Wash);
        let view = crate::view::View {
            zoom: 2.0,
            angle: 37.0,
            pan: [12.0, -8.0],
            flipped: true,
        };
        storage.save_view(view).unwrap();
        assert_eq!(storage.load_view().unwrap(), view);
        fs::write(root.join("settings/preferences.tmp"), "interrupted write").unwrap();
        assert!(storage.save_brush(BrushTip::Pencil).is_err());
        assert_eq!(storage.load_brush().unwrap(), BrushTip::Wash);
        fs::remove_file(root.join("settings/preferences.tmp")).unwrap();
        fs::write(
            root.join("settings/preferences.txt"),
            "version=99\nbrush=invalid\n",
        )
        .unwrap();
        assert_eq!(
            storage.load_brush().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(root).unwrap();
    }
}
