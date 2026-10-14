use crate::{scene::Point, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filter {
    Nearest,
    Bilinear,
    Bicubic,
    Area,
    Oversample,
}
impl Filter {
    pub const ALL: [Self; 5] = [
        Self::Nearest,
        Self::Bilinear,
        Self::Bicubic,
        Self::Area,
        Self::Oversample,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Nearest => "Hard edges (nearest)",
            Self::Bilinear => "Smooth edges (bilinear)",
            Self::Bicubic => "Clear edges (bicubic)",
            Self::Area => "High accuracy (average colors)",
            Self::Oversample => "Smooth (4×4 oversampling)",
        }
    }
}
pub fn box_footprint(point: Point, step: Point) -> [Point; 4] {
    let [x, y] = point;
    let [w, h] = step;
    [
        [x - w / 2.0, y - h / 2.0],
        [x + w / 2.0, y - h / 2.0],
        [x + w / 2.0, y + h / 2.0],
        [x - w / 2.0, y + h / 2.0],
    ]
}
fn accumulate(sum: &mut [f64; 4], p: [u8; 4], weight: f64) {
    let alpha = p[3] as f64 / 255.0;
    for i in 0..3 {
        sum[i] += p[i] as f64 * alpha * weight;
    }
    sum[3] += alpha * weight;
}
fn color(sum: [f64; 4]) -> [u8; 4] {
    if sum[3] <= 1e-12 {
        return [0; 4];
    }
    [
        (sum[0] / sum[3]).round().clamp(0.0, 255.0) as u8,
        (sum[1] / sum[3]).round().clamp(0.0, 255.0) as u8,
        (sum[2] / sum[3]).round().clamp(0.0, 255.0) as u8,
        (sum[3] * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}
fn cubic(t: f64) -> f64 {
    let t = t.abs();
    if t <= 1.0 {
        1.5 * t * t * t - 2.5 * t * t + 1.0
    } else if t < 2.0 {
        -0.5 * t * t * t + 2.5 * t * t - 4.0 * t + 2.0
    } else {
        0.0
    }
}
fn linear_sum(point: Point, at: &impl Fn(i64, i64) -> [u8; 4]) -> [f64; 4] {
    let x = point[0].floor() as i64;
    let y = point[1].floor() as i64;
    let fx = point[0] - x as f64;
    let fy = point[1] - y as f64;
    let mut sum = [0.0; 4];
    for (dx, dy, w) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        accumulate(&mut sum, at(x + dx, y + dy), w);
    }
    sum
}
fn area(poly: &[Point]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let origin = poly[0];
    poly.iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
        .map(|(a, b)| {
            (a[0] - origin[0]) * (b[1] - origin[1]) - (a[1] - origin[1]) * (b[0] - origin[0])
        })
        .sum::<f64>()
        .abs()
        / 2.0
}
fn clipped(poly: &[Point], axis: usize, bound: f64, greater: bool) -> Vec<Point> {
    let mut out = Vec::new();
    if poly.is_empty() {
        return out;
    }
    let inside = |p: Point| {
        if greater {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let mut a = *poly.last().unwrap();
    for &b in poly {
        let ai = inside(a);
        let bi = inside(b);
        if ai != bi {
            let t = (bound - a[axis]) / (b[axis] - a[axis]);
            let mut p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            p[axis] = bound;
            out.push(p);
        }
        if bi {
            out.push(b);
        }
        a = b;
    }
    out
}
/// Coordinates are pixel-center indices; footprint is the inverse-mapped output pixel quad.
pub fn sample(
    point: Point,
    footprint: [Point; 4],
    filter: Filter,
    at: impl Fn(i64, i64) -> [u8; 4],
) -> [u8; 4] {
    if point
        .iter()
        .chain(footprint.iter().flatten())
        .any(|n| !n.is_finite() || n.abs() > 1e13)
    {
        return [0; 4];
    }
    match filter {
        Filter::Nearest => at(point[0].round() as i64, point[1].round() as i64),
        Filter::Bilinear => color(linear_sum(point, &at)),
        Filter::Bicubic => {
            let x = point[0].floor() as i64;
            let y = point[1].floor() as i64;
            let mut sum = [0.0; 4];
            for dy in -1..=2 {
                for dx in -1..=2 {
                    accumulate(
                        &mut sum,
                        at(x + dx, y + dy),
                        cubic(point[0] - (x + dx) as f64) * cubic(point[1] - (y + dy) as f64),
                    );
                }
            }
            color(sum)
        }
        Filter::Oversample => {
            let mut sum = [0.0; 4];
            for y in 0..4 {
                for x in 0..4 {
                    let u = (x as f64 + 0.5) / 4.0;
                    let v = (y as f64 + 0.5) / 4.0;
                    let p = [
                        footprint[0][0]
                            + u * (footprint[1][0] - footprint[0][0])
                            + v * (footprint[3][0] - footprint[0][0]),
                        footprint[0][1]
                            + u * (footprint[1][1] - footprint[0][1])
                            + v * (footprint[3][1] - footprint[0][1]),
                    ];
                    let s = linear_sum(p, &at);
                    for i in 0..4 {
                        sum[i] += s[i] / 16.0;
                    }
                }
            }
            color(sum)
        }
        Filter::Area => {
            let total = area(&footprint);
            if total < 1e-12 {
                return at(point[0].round() as i64, point[1].round() as i64);
            }
            let min = [
                footprint.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
                footprint.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
            ];
            let max = [
                footprint
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::NEG_INFINITY, f64::max),
                footprint
                    .iter()
                    .map(|p| p[1])
                    .fold(f64::NEG_INFINITY, f64::max),
            ];
            let mut sum = [0.0; 4];
            let aligned = ((max[0] - min[0]) * (max[1] - min[1]) - total).abs() < total * 1e-8;
            for y in (min[1] + 0.5).floor() as i64..=(max[1] + 0.5).ceil() as i64 {
                for x in (min[0] + 0.5).floor() as i64..=(max[0] + 0.5).ceil() as i64 {
                    let weight = if aligned {
                        ((max[0].min(x as f64 + 0.5) - min[0].max(x as f64 - 0.5)).max(0.0)
                            * (max[1].min(y as f64 + 0.5) - min[1].max(y as f64 - 0.5)).max(0.0))
                            / total
                    } else {
                        let p = clipped(&footprint, 0, x as f64 - 0.5, true);
                        let p = clipped(&p, 0, x as f64 + 0.5, false);
                        let p = clipped(&p, 1, y as f64 - 0.5, true);
                        let p = clipped(&p, 1, y as f64 + 0.5, false);
                        area(&p) / total
                    };
                    if weight > 0.0 {
                        accumulate(&mut sum, at(x, y), weight);
                    }
                }
            }
            color(sum)
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    View,
    Resolution,
    Object,
    Animation,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub view: Filter,
    pub resolution: Filter,
    pub object: Filter,
    pub animation: Filter,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            view: Filter::Nearest,
            resolution: Filter::Bilinear,
            object: Filter::Bilinear,
            animation: Filter::Bilinear,
        }
    }
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
    if let Err(e) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(e);
    }
    Ok(())
}
impl Settings {
    pub fn filter(self, scope: Scope) -> Filter {
        match scope {
            Scope::View => self.view,
            Scope::Resolution => self.resolution,
            Scope::Object => self.object,
            Scope::Animation => self.animation,
        }
    }
    pub fn sample(
        self,
        scope: Scope,
        point: Point,
        footprint: [Point; 4],
        at: impl Fn(i64, i64) -> [u8; 4],
    ) -> [u8; 4] {
        sample(point, footprint, self.filter(scope), at)
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Interpolation settings too large"));
        }
        let s: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if s.version != 1 {
            return Err(io::Error::other("Unsupported interpolation settings"));
        }
        Ok(s)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match Self::read(&storage.root().join("settings/interpolation.json")) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(self, storage: &UserStorage) -> io::Result<()> {
        if self.version != 1 {
            return Err(io::Error::other("Invalid interpolation settings"));
        }
        let path = storage.root().join("settings/interpolation.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(io::Error::other(
                    "Interpolation settings exceed recovery limit",
                ));
            }
            let name = if Self::read(&path).is_ok() {
                "interpolation.previous.json"
            } else {
                "interpolation.damaged.json"
            };
            write(
                &storage.root().join("settings").join(name),
                &fs::read(&path)?,
            )?;
        }
        write(&path, &serde_json::to_vec(&self).map_err(io::Error::other)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
        sizing::{self, Change},
        test_support::TempDir,
    };
    use std::sync::atomic::AtomicBool;
    #[test]
    fn checker_reduction_alpha_rotated_footprints_and_distinct_oversampling() {
        let checker = |x: i64, y: i64| {
            if (x + y).rem_euclid(2) == 0 {
                [255; 4]
            } else {
                [0, 0, 0, 255]
            }
        };
        let fp = box_footprint([1.5, 1.5], [4.0, 4.0]);
        assert_eq!(sample([1.5, 1.5], fp, Filter::Nearest, checker), [255; 4]);
        assert_eq!(
            sample([1.5, 1.5], fp, Filter::Area, checker),
            [128, 128, 128, 255]
        );
        let stripe = |x: i64, _| if x == 2 { [255; 4] } else { [0, 0, 0, 255] };
        let fp = box_footprint([3.0, 0.0], [7.0, 1.0]);
        assert_eq!(
            sample([3.0, 0.0], fp, Filter::Area, stripe),
            [36, 36, 36, 255]
        );
        assert_eq!(
            sample([3.0, 0.0], fp, Filter::Oversample, stripe),
            [56, 56, 56, 255]
        );
        for angle in [0.0f64, 37.0, 90.0] {
            let (sin, cos) = angle.to_radians().sin_cos();
            let fp = box_footprint([0.0, 0.0], [0.4, 1.3]).map(|p| {
                [
                    p[0] * cos - p[1] * sin + 10.0,
                    p[0] * sin + p[1] * cos + 10.0,
                ]
            });
            for filter in Filter::ALL {
                assert_eq!(
                    sample([10.0, 10.0], fp, filter, |_, _| [200, 100, 50, 128]),
                    [200, 100, 50, 128]
                );
            }
        }
        let edge = |x: i64, _| {
            if x <= 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 0]
            }
        };
        for filter in [
            Filter::Bilinear,
            Filter::Bicubic,
            Filter::Area,
            Filter::Oversample,
        ] {
            assert_eq!(
                sample(
                    [0.5, 0.0],
                    box_footprint([0.5, 0.0], [1.0, 1.0]),
                    filter,
                    edge
                ),
                [255, 0, 0, 128]
            );
        }
    }
    #[test]
    fn settings_scopes_backup_and_all_resolution_filters_keep_source_unchanged() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut settings = Settings::default();
        settings.save(&storage).unwrap();
        settings.view = Filter::Bicubic;
        settings.object = Filter::Area;
        settings.animation = Filter::Oversample;
        settings.save(&storage).unwrap();
        assert_eq!(Settings::load(&storage).unwrap(), settings);
        assert_eq!(
            Settings::read(&storage.root().join("settings/interpolation.previous.json")).unwrap(),
            Settings::default()
        );
        assert_eq!(settings.filter(Scope::Object), Filter::Area);
        assert_eq!(settings.filter(Scope::Animation), Filter::Oversample);
        assert_eq!(settings.filter(Scope::Resolution), Filter::Bilinear);
        let fp = box_footprint([3.0, 0.0], [7.0, 1.0]);
        let stripe = |x: i64, _| if x == 2 { [255; 4] } else { [0, 0, 0, 255] };
        assert_eq!(
            settings.sample(Scope::Object, [3.0, 0.0], fp, stripe),
            [36, 36, 36, 255]
        );
        assert_eq!(
            settings.sample(Scope::Animation, [3.0, 0.0], fp, stripe),
            [56, 56, 56, 255]
        );
        let mut doc = Document::create(
            CanvasSpec {
                width: 4,
                height: 4,
                dpi: 300.0,
                background: Background::Transparent,
            },
            1000,
        )
        .unwrap();
        for (i, p) in doc.layers[0].pixels.iter_mut().enumerate() {
            *p = if (i % 4 + i / 4) % 2 == 0 {
                [255; 4]
            } else {
                [0, 0, 0, 255]
            };
        }
        let original = doc.snapshot();
        for filter in Filter::ALL {
            let output = sizing::prepare(
                &original,
                Change::Resample {
                    width: 2,
                    height: 2,
                    dpi: 300.0,
                    filter,
                },
                DEFAULT_DOCUMENT_BUDGET,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!((output.width, output.height), (2, 2));
            assert_eq!(doc.snapshot(), original);
        }
        fs::write(
            storage.root().join("settings/interpolation.json.write.tmp"),
            "foreign",
        )
        .unwrap();
        assert!(Settings::default().save(&storage).is_err());
        assert_eq!(Settings::load(&storage).unwrap(), settings);
    }
}
