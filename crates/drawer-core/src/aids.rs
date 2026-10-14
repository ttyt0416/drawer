use crate::{
    scene::{Affine, Point},
    storage::UserStorage,
    view::View,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = path
        .file_name()
        .ok_or_else(|| io::Error::other("Missing settings filename"))?
        .to_owned();
    name.push(".write.tmp");
    let temporary = path.with_file_name(name);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(e) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(e);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Pixels,
    Millimeters,
    Inches,
}
impl Unit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pixels => "px",
            Self::Millimeters => "mm",
            Self::Inches => "in",
        }
    }
    pub fn to_pixels(self, value: f64, dpi: f64) -> f64 {
        match self {
            Self::Pixels => value,
            Self::Millimeters => value * dpi / 25.4,
            Self::Inches => value * dpi,
        }
    }
    pub fn from_pixels(self, value: f64, dpi: f64) -> f64 {
        match self {
            Self::Pixels => value,
            Self::Millimeters => value * 25.4 / dpi,
            Self::Inches => value / dpi,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub spacing: Point,
    pub origin: Point,
}
impl Default for Grid {
    fn default() -> Self {
        Self {
            spacing: [64.0; 2],
            origin: [0.0; 2],
        }
    }
}
impl Grid {
    pub fn valid(self) -> bool {
        self.spacing
            .iter()
            .all(|v| v.is_finite() && *v > 0.0 && *v <= 1e6)
            && self.origin.iter().all(|v| v.is_finite() && v.abs() <= 1e12)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AidPreferences {
    pub version: u32,
    pub unit: Unit,
    pub rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub monitor_ppi: Option<f64>,
}
impl Default for AidPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            unit: Unit::Pixels,
            rulers: true,
            grid: false,
            guides: true,
            monitor_ppi: None,
        }
    }
}
impl AidPreferences {
    pub fn valid(self) -> bool {
        self.version == 1
            && self
                .monitor_ppi
                .is_none_or(|v| v.is_finite() && (20.0..=1000.0).contains(&v))
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        let path = storage.root().join("settings/aids.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        if fs::metadata(&path)?.len() > 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Canvas aid settings are too large",
            ));
        }
        let preferences: Self = serde_json::from_slice(&fs::read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if !preferences.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported canvas aid settings",
            ));
        }
        Ok(preferences)
    }
    pub fn save(self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid canvas aid settings",
            ));
        }
        let path = storage.root().join("settings/aids.json");
        if path.exists() {
            let backup = if Self::load(storage).is_ok() {
                "aids.previous.json"
            } else {
                "aids.damaged.json"
            };
            if fs::metadata(&path)?.len() > 8 * 65536 {
                return Err(io::Error::other(
                    "Canvas aid settings exceed the recovery budget; the original is preserved",
                ));
            }
            atomic_write(
                &storage.root().join("settings").join(backup),
                &fs::read(&path)?,
            )?;
        }
        atomic_write(&path, &serde_json::to_vec(&self).map_err(io::Error::other)?)
    }
    pub fn previous(storage: &UserStorage) -> io::Result<Self> {
        let path = storage.root().join("settings/aids.previous.json");
        if fs::metadata(&path)?.len() > 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Canvas aid backup is too large",
            ));
        }
        let p: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if !p.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid canvas aid backup",
            ));
        }
        Ok(p)
    }
}
pub fn calibrated_ppi(logical_length: f64, ppp: f64, measured_mm: f64) -> Option<f64> {
    if !logical_length.is_finite()
        || logical_length <= 0.0
        || !ppp.is_finite()
        || ppp <= 0.0
        || !measured_mm.is_finite()
        || measured_mm <= 0.0
    {
        return None;
    }
    let ppi = logical_length * ppp * 25.4 / measured_mm;
    (20.0..=1000.0).contains(&ppi).then_some(ppi)
}
pub fn print_zoom(dpi: f64, ppi: f64) -> Option<f64> {
    let zoom = ppi / dpi;
    (dpi.is_finite()
        && dpi > 0.0
        && ppi.is_finite()
        && (20.0..=1000.0).contains(&ppi)
        && (0.01..=64.0).contains(&zoom))
    .then_some(zoom)
}
pub struct Tick {
    pub screen: f64,
    pub value: f64,
}
pub struct Ruler {
    pub axis: usize,
    pub ticks: Vec<Tick>,
}
pub fn ruler(
    view: View,
    size: Point,
    viewport: [Point; 2],
    ppp: f64,
    dpi: f64,
    unit: Unit,
    horizontal: bool,
) -> Ruler {
    let [min, max] = viewport;
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let from = view.screen_to_document(min, size, center, ppp);
    let to = view.screen_to_document([max[0], min[1]], size, center, ppp);
    let haxis = usize::from((to[1] - from[1]).abs() > (to[0] - from[0]).abs());
    let axis = if horizontal { haxis } else { 1 - haxis };
    let end = if horizontal {
        [max[0], min[1]]
    } else {
        [min[0], max[1]]
    };
    let end = view.screen_to_document(end, size, center, ppp);
    let first = unit.from_pixels(from[axis], dpi);
    let last = unit.from_pixels(end[axis], dpi);
    let span = (last - first).abs();
    if !span.is_finite() || span < 1e-10 {
        return Ruler {
            axis,
            ticks: Vec::new(),
        };
    }
    let logical = if horizontal {
        max[0] - min[0]
    } else {
        max[1] - min[1]
    };
    let desired = span * 60.0 / logical.max(1.0);
    if !desired.is_finite() || desired <= 0.0 {
        return Ruler {
            axis,
            ticks: Vec::new(),
        };
    }
    let power = 10.0f64.powf(desired.log10().floor());
    let factor = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .find(|f| power * f >= desired)
        .unwrap_or(10.0);
    let step = power * factor;
    let begin = (first.min(last) / step).ceil() * step;
    let mut ticks = Vec::new();
    for n in 0..256 {
        let value = begin + n as f64 * step;
        if value > first.max(last) + step * 1e-9 {
            break;
        }
        let t = (value - first) / (last - first);
        let screen = if horizontal {
            min[0] + t * (max[0] - min[0])
        } else {
            min[1] + t * (max[1] - min[1])
        };
        ticks.push(Tick { screen, value });
    }
    Ruler { axis, ticks }
}

pub fn grid_lines(
    view: View,
    transform: Affine,
    grid: Grid,
    size: Point,
    viewport: [Point; 2],
    ppp: f64,
) -> Vec<[Point; 2]> {
    let center = [
        (viewport[0][0] + viewport[1][0]) / 2.0,
        (viewport[0][1] + viewport[1][1]) / 2.0,
    ];
    let original = crate::navigator::visible_polygon(view, size, viewport, ppp)
        .map(|p| transform.inverse().map(p));
    let mut min = original[0];
    let mut max = original[0];
    for point in original {
        for i in 0..2 {
            min[i] = min[i].min(point[i]);
            max[i] = max[i].max(point[i]);
        }
    }
    let mut lines = Vec::new();
    for axis in 0..2 {
        let first = ((min[axis] - grid.origin[axis]) / grid.spacing[axis]).floor();
        let last = ((max[axis] - grid.origin[axis]) / grid.spacing[axis]).ceil();
        let skip = ((last - first) / 128.0).ceil().max(1.0);
        for n in 0..=128 {
            let index = first + n as f64 * skip;
            if index > last {
                break;
            }
            let coordinate = grid.origin[axis] + index * grid.spacing[axis];
            let other = 1 - axis;
            let mut a = min;
            let mut b = max;
            a[axis] = coordinate;
            b[axis] = coordinate;
            a[other] = min[other];
            b[other] = max[other];
            let line = [a, b].map(|p| view.document_to_screen(transform.map(p), size, center, ppp));
            if let Some(line) = clip_line(line, viewport) {
                lines.push(line);
            }
        }
    }
    lines
}
pub fn clip_line([start, end]: [Point; 2], [min, max]: [Point; 2]) -> Option<[Point; 2]> {
    if start.iter().chain(&end).any(|v| !v.is_finite()) {
        return None;
    }
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for i in 0..2 {
        if delta[i].abs() < 1e-12 {
            if start[i] < min[i] || start[i] > max[i] {
                return None;
            }
        } else {
            let a = (min[i] - start[i]) / delta[i];
            let b = (max[i] - start[i]) / delta[i];
            low = low.max(a.min(b));
            high = high.min(a.max(b));
        }
    }
    if low > high {
        return None;
    }
    Some([low, high].map(|t| {
        [
            (start[0] + delta[0] * t).clamp(min[0], max[0]),
            (start[1] + delta[1] * t).clamp(min[1], max[1]),
        ]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec, Document},
        files,
        test_support::TempDir,
    };
    #[test]
    fn units_and_user_calibration_preserve_physical_lengths_at_different_display_scaling() {
        for dpi in [150.0, 300.0, 600.0] {
            assert!(
                (Unit::Millimeters.to_pixels(25.4, dpi) - Unit::Inches.to_pixels(1.0, dpi)).abs()
                    < 1e-9
            );
            for ppp in [1.0, 1.5, 2.0] {
                let measured = 200.0 * ppp * 25.4 / 96.0;
                let ppi = calibrated_ppi(200.0, ppp, measured).unwrap();
                let zoom = print_zoom(dpi, ppi).unwrap();
                let view = View {
                    zoom,
                    ..Default::default()
                };
                let a = view.document_to_screen([0.0, 0.0], [dpi, dpi], [0.0; 2], ppp);
                let b = view.document_to_screen([dpi, 0.0], [dpi, dpi], [0.0; 2], ppp);
                assert!(((b[0] - a[0]) * ppp / ppi - 1.0).abs() < 1e-9);
            }
        }
        for mm in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-8] {
            assert!(calibrated_ppi(200.0, 1.0, mm).is_none());
        }
    }
    #[test]
    fn rulers_follow_document_coordinates_and_grid_is_bounded_at_extreme_zoom() {
        let size = [1200.0, 900.0];
        let viewport = [[200.0, 100.0], [1000.0, 700.0]];
        let center = [600.0, 400.0];
        for unit in [Unit::Pixels, Unit::Millimeters, Unit::Inches] {
            for zoom in [0.01, 1.0, 64.0] {
                for angle in [0.0, 37.0, 90.0] {
                    for flipped in [false, true] {
                        let view = View {
                            zoom,
                            angle,
                            flipped,
                            pan: [120.0, -75.0],
                        };
                        for horizontal in [true, false] {
                            let result = ruler(view, size, viewport, 1.5, 300.0, unit, horizontal);
                            assert!(!result.ticks.is_empty());
                            for tick in result.ticks {
                                let point = if horizontal {
                                    [tick.screen, viewport[0][1]]
                                } else {
                                    [viewport[0][0], tick.screen]
                                };
                                let doc = view.screen_to_document(point, size, center, 1.5);
                                assert!(
                                    (unit.to_pixels(tick.value, 300.0) - doc[result.axis]).abs()
                                        < 1e-6
                                );
                            }
                        }
                        let lines = grid_lines(
                            view,
                            Affine::default(),
                            Grid::default(),
                            size,
                            viewport,
                            1.5,
                        );
                        assert!(lines.len() <= 258);
                        assert!(lines.iter().flatten().all(|p| p[0] >= 200.0
                            && p[0] <= 1000.0
                            && p[1] >= 100.0
                            && p[1] <= 700.0));
                    }
                }
            }
        }
    }
    #[test]
    fn grid_and_guides_save_as_one_metadata_command_undo_and_keep_raster_pixels() {
        let root = TempDir::new();
        let mut doc = Document::create(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 300.0,
                background: Background::Color([10, 20, 30]),
            },
            8,
        )
        .unwrap();
        let original = doc.snapshot();
        doc.mark_saved(root.0.join("file.drawer"));
        let mut scene = doc.scene.clone();
        scene.grid = Grid {
            spacing: [25.0, 50.0],
            origin: [10.0, 0.0],
        };
        scene.guides.push([[0.0, 0.5], [2.0, 0.5]]);
        doc.edit_scene(scene).unwrap();
        assert!(doc.dirty);
        assert_eq!(doc.layers, original.layers);
        assert_eq!(doc.history_counts(), (1, 0));
        let path = root.0.join("result.drawer");
        files::save(&mut doc, &path).unwrap();
        let reopened = files::open(&path, 1024).unwrap();
        assert_eq!(doc.snapshot(), reopened.snapshot());
        assert!(!doc.dirty);
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), original);
        assert!(doc.dirty);
        assert!(doc.redo());
        assert!(!doc.dirty);
        let before = doc.snapshot();
        let mut invalid = doc.scene.clone();
        invalid.grid.spacing[0] = 0.0;
        assert!(doc.edit_scene(invalid).is_err());
        assert_eq!(doc.snapshot(), before);
    }
    #[test]
    fn view_preferences_back_up_and_failed_write_preserves_previous_settings() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.clone()).unwrap();
        assert_eq!(
            AidPreferences::load(&storage).unwrap(),
            AidPreferences::default()
        );
        AidPreferences::default().save(&storage).unwrap();
        let prefs = AidPreferences {
            grid: true,
            unit: Unit::Millimeters,
            monitor_ppi: Some(96.0),
            ..Default::default()
        };
        prefs.save(&storage).unwrap();
        assert_eq!(AidPreferences::load(&storage).unwrap(), prefs);
        assert_eq!(
            AidPreferences::previous(&storage).unwrap(),
            AidPreferences::default()
        );
        let temp = root.0.join("settings/aids.json.write.tmp");
        fs::write(&temp, "busy").unwrap();
        assert!(AidPreferences::default().save(&storage).is_err());
        assert_eq!(AidPreferences::load(&storage).unwrap(), prefs);
        assert_eq!(fs::read_to_string(temp).unwrap(), "busy");
    }
}
