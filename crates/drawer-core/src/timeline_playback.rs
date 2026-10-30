//! Local preview scheduling and disposable, bounded precomputed frame images.
use crate::{
    animation::Fps,
    document::Document,
    timeline_view::{Onion, Preview},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    RealTime,
    EveryFrame,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    #[default]
    Quality,
    Speed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub mode: Mode,
    pub quality: Quality,
    pub camera: bool,
    pub fps: Option<Fps>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            mode: Mode::RealTime,
            quality: Quality::Quality,
            camera: true,
            fps: None,
        }
    }
}
impl Preferences {
    pub fn valid(self) -> bool {
        self.version == 1 && self.fps.is_none_or(Fps::valid)
    }
    pub fn side(self) -> u32 {
        match self.quality {
            Quality::Quality => 256,
            Quality::Speed => 96,
        }
    }
    pub fn rate(self, d: &Document) -> Result<Fps, String> {
        if !self.valid() {
            return Err("Invalid local playback options".into());
        }
        Ok(self
            .fps
            .unwrap_or(d.animation.as_ref().ok_or("Choose a timeline")?.fps))
    }
    pub fn silent(self, d: &Document) -> bool {
        self.mode == Mode::EveryFrame
            || self.fps.is_some_and(|f| {
                d.animation.as_ref().is_some_and(|a| {
                    f.numerator as u64 * a.fps.denominator as u64
                        != a.fps.numerator as u64 * f.denominator as u64
                })
            })
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local playback settings storage".into());
        }
        let p = root.join("settings/timeline-playback.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let bytes = crate::book_io::read_local(&p, 8192)?;
        let s: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !s.valid() {
            return Err("Invalid playback preferences; file preserved".into());
        }
        Ok(s)
    }
    pub fn save(self, root: &Path) -> Result<(), String> {
        if !crate::font_library::local(root) {
            return Err("Choose local playback settings storage".into());
        }
        if !self.valid() {
            return Err("Use a valid rational playback FPS within240fps".into());
        }
        let parent = root.join("settings");
        let mut stage = crate::animation_export::Stage::new(&parent)?;
        let bytes = serde_json::to_vec_pretty(&self).map_err(|e| e.to_string())?;
        let mut f = stage.create("playback.json")?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(
            stage.directory.join("playback.json"),
            parent.join("timeline-playback.json"),
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub first: u32,
    pub end: u32,
    pub repeat: bool,
}
impl Range {
    pub fn of(d: &Document, fallback: bool) -> Result<Self, String> {
        let a = d.animation.as_ref().ok_or("Choose a timeline")?;
        let p = d
            .scene
            .timing
            .playback
            .unwrap_or(crate::timeline_timing::Playback {
                first: 0,
                end: a.frames,
                repeat: fallback,
            });
        if p.first >= p.end || p.end > a.frames {
            return Err("Invalid playback range".into());
        }
        Ok(Self {
            first: p.first,
            end: p.end,
            repeat: p.repeat,
        })
    }
}
pub struct Cursor {
    range: Range,
    fps: Fps,
    mode: Mode,
    start: u32,
    frame: u32,
    epoch: f64,
    next: f64,
    last_pass: Option<u64>,
    done: bool,
}
impl Cursor {
    pub fn new(range: Range, start: u32, fps: Fps, mode: Mode, now: f64) -> Result<Self, String> {
        if range.first >= range.end || range.end > 10000 || !fps.valid() || !now.is_finite() {
            return Err("Invalid playback clock/range/FPS".into());
        }
        let start = if (range.first..range.end).contains(&start) {
            start
        } else {
            range.first
        };
        Ok(Self {
            range,
            fps,
            mode,
            start,
            frame: start,
            epoch: now,
            next: now + 1. / fps.value(),
            last_pass: None,
            done: false,
        })
    }
    /// EveryFrame advances at most once after a rendered UI pass; late rendering slows playback.
    pub fn step(&mut self, now: f64, pass: u64) -> (u32, bool) {
        if self.done || !now.is_finite() || now < self.epoch {
            return (self.frame, self.done);
        }
        if self.mode == Mode::RealTime {
            let (frame, done) = crate::timeline_editor::playback(
                self.start - self.range.first,
                (now - self.epoch).max(0.),
                self.fps,
                self.range.end - self.range.first,
                self.range.repeat,
            )
            .unwrap();
            self.frame = frame + self.range.first;
            self.done = done;
        } else if self.last_pass.is_some_and(|old| pass > old) && now >= self.next {
            if self.frame + 1 < self.range.end {
                self.frame += 1;
            } else if self.range.repeat {
                self.frame = self.range.first;
            } else {
                self.done = true;
            }
            self.next = now + 1. / self.fps.value();
            self.last_pass = None;
        }
        (self.frame, self.done)
    }
    pub fn presented(&mut self, pass: u64) {
        self.last_pass = Some(pass);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub track: Option<u64>,
    pub only: bool,
    pub onion: Onion,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub document: u64,
    pub revision: u64,
    pub timeline: u64,
    pub content: Option<crate::keyframes::Owner>,
    pub range: Range,
    pub view: View,
    pub preferences: Preferences,
}
impl Stamp {
    pub fn of(d: &Document, view: View, p: Preferences, repeat: bool) -> Result<Self, String> {
        if !p.valid() {
            return Err("Invalid playback options".into());
        }
        Ok(Self {
            document: d.id,
            revision: d.revision,
            timeline: d.scene.timelines.active,
            content: d.keyframe_content,
            range: Range::of(d, repeat)?,
            view,
            preferences: p,
        })
    }
}
pub struct Cache {
    pub stamp: Stamp,
    pub frames: Vec<Preview>,
    pub bytes: usize,
}
impl Cache {
    pub fn get(&self, frame: u32) -> Option<&Preview> {
        let n = frame.checked_sub(self.stamp.range.first)?;
        self.frames.get(n as usize)
    }
    pub fn build(
        d: &Document,
        stamp: Stamp,
        budget: usize,
        cancel: &AtomicBool,
        progress: &AtomicU32,
    ) -> Result<Self, String> {
        if stamp != Stamp::of(d, stamp.view, stamp.preferences, stamp.range.repeat)? {
            return Err("Preview capture is stale".into());
        }
        let side = stamp.preferences.side();
        let scale = (d.width.max(d.height) as f64 / side as f64).max(1.);
        let w = (d.width as f64 / scale).ceil() as usize;
        let h = (d.height as f64 / scale).ceil() as usize;
        let bytes = w
            .saturating_mul(h)
            .saturating_mul(8)
            .saturating_mul((stamp.range.end - stamp.range.first) as usize);
        let working = d
            .memory_bytes()
            .saturating_mul(stamp.view.onion.before.max(stamp.view.onion.after) as usize * 2 + 4);
        if bytes > 64 * 1024 * 1024 || bytes.saturating_add(working) > budget {
            return Err("Precompute exceeds64MiB cache or available working memory; choose speed quality or a shorter playback range".into());
        }
        let mut frozen = crate::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.keyframe_content = d.keyframe_content;
        if !stamp.preferences.camera {
            frozen.scene.keyframes.camera_output = false;
        }
        let mut frames = Vec::new();
        frames
            .try_reserve_exact((stamp.range.end - stamp.range.first) as usize)
            .map_err(|e| e.to_string())?;
        for f in stamp.range.first..stamp.range.end {
            crate::book_io::check_cancel(cancel)?;
            frozen.current_frame = f;
            frames.push(crate::timeline_view::preview_cancellable(
                &frozen,
                stamp.view.track,
                stamp.view.only,
                stamp.view.onion,
                side,
                budget.saturating_sub(bytes),
                cancel,
            )?);
            progress.store(frames.len() as u32, Ordering::Relaxed);
        }
        crate::book_io::check_cancel(cancel)?;
        Ok(Self {
            stamp,
            frames,
            bytes,
        })
    }
}
pub fn preview(
    d: &Document,
    view: View,
    p: Preferences,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Preview, String> {
    if p.camera || !d.scene.keyframes.camera_output {
        return crate::timeline_view::preview_cancellable(
            d,
            view.track,
            view.only,
            view.onion,
            p.side(),
            budget,
            cancel,
        );
    }
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Camera preview exceeds the working memory budget".into());
    }
    let mut frozen = crate::layer_merge::preview(&d.snapshot(), d.current_frame)?;
    frozen.keyframe_content = d.keyframe_content;
    frozen.scene.keyframes.camera_output = false;
    crate::timeline_view::preview_cancellable(
        &frozen,
        view.track,
        view.only,
        view.onion,
        p.side(),
        budget,
        cancel,
    )
}
pub fn verify(root: &Path) -> Result<(), String> {
    use crate::{
        animation_export::{self, Options},
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
        keyframes::{Curve, Edit, Keyframe, Owner, Pose},
    };
    let dir = root.join("timeline-playback-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        let mut d = Document::create(
            CanvasSpec {
                width: 300,
                height: 4,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        for y in 0..4 {
            for x in 0..32 {
                d.edit_pixel(0, x, y, [255, 0, 0, 255]);
            }
        }
        d.commit_state(crate::timeline_editor::create(&d, 4, Fps::default())?)?;
        d.commit_state(crate::camera2d::enable(&d, true)?)?;
        d.commit_state(crate::keyframes::prepare(
            &d,
            Owner::Camera,
            Edit::Put {
                key: Keyframe {
                    frame: 0,
                    pose: Pose {
                        offset: [64., 0.],
                        pivot: [150., 2.],
                        ..Default::default()
                    },
                    curve: Curve::Fixed,
                },
                replace: true,
            },
        )?)?;
        let original = d.snapshot();
        let counts = d.history_counts();
        let dirty = d.dirty;
        let capture = animation_export::prepare(
            &d,
            Options {
                first: 0,
                last: 3,
                ..Default::default()
            },
            B,
        )?;
        let output_before = crate::animation_render::frame(&capture, 0, &cancel)?;
        let view = View {
            track: None,
            only: false,
            onion: Default::default(),
        };
        let prefs = Preferences::default();
        let stamp = Stamp::of(&d, view, prefs, false)?;
        let cache = Cache::build(&d, stamp.clone(), B, &cancel, &progress)?;
        check(
            cache.frames.len() == 4
                && progress.load(Ordering::Relaxed) == 4
                && cache.get(3).is_some()
                && cache.get(4).is_none(),
            "Precompute omitted or added frames",
        )?;
        let live = preview(&d, view, prefs, B, &cancel)?;
        check(
            live.image.pixels == cache.get(0).unwrap().image.pixels,
            "Precomputed quality preview differs from live rendering",
        )?;
        let base = Preferences {
            camera: false,
            quality: Quality::Speed,
            mode: Mode::EveryFrame,
            fps: Some(Fps {
                numerator: 12,
                denominator: 1,
            }),
            ..prefs
        };
        let base_cache =
            Cache::build(&d, Stamp::of(&d, view, base, false)?, B, &cancel, &progress)?;
        check(
            base_cache.bytes < cache.bytes
                && base_cache.get(0).unwrap().image.pixels[0][0] > 240
                && cache.get(0).unwrap().image.pixels[0][3] == 0,
            "Preview camera/quality policies were not applied",
        )?;
        let after = animation_export::prepare(
            &d,
            Options {
                first: 0,
                last: 3,
                ..Default::default()
            },
            B,
        )?;
        check(
            crate::animation_render::frame(&after, 0, &cancel)? == output_before
                && d.snapshot() == original
                && d.history_counts() == counts
                && d.dirty == dirty,
            "Preview changed final output, drawing or Undo",
        )?;
        let outcome =
            animation_export::publish(&after, &dir.join("frames"), false, &cancel, &progress);
        check(
            outcome.error.is_none()
                && outcome.frames == 4
                && outcome
                    .files
                    .iter()
                    .filter(|p| p.extension().is_some_and(|e| e == "png"))
                    .count()
                    == 4,
            "Output omitted frames under every-frame preview options",
        )?;
        check(
            Cache::build(&d, stamp.clone(), 1, &cancel, &progress).is_err(),
            "Precompute working-memory limit ignored",
        )?;
        check(
            Cache::build(&d, stamp.clone(), B, &AtomicBool::new(true), &progress).is_err()
                && d.snapshot() == original,
            "Cancelled precompute changed drawing",
        )?;
        let mut invalid = stamp.clone();
        invalid.range.end = 0;
        check(
            Cache::build(&d, invalid, B, &cancel, &progress).is_err(),
            "Invalid cache range accepted",
        )?;
        d.current_frame = 3;
        check(
            Stamp::of(&d, view, prefs, false)? == stamp,
            "Playhead movement needlessly invalidated cached source",
        )?;
        d.commit_state(crate::camera2d::enable(&d, false)?)?;
        check(
            d.edit_pixel(0, 100, 0, [0, 255, 0, 255]),
            "Source edit fixture failed",
        )?;
        check(
            Stamp::of(&d, view, prefs, false)? != stamp
                && Cache::build(&d, stamp, B, &cancel, &progress).is_err(),
            "Changed source accepted old cache",
        )?;
        let mut s = d.snapshot();
        s.scene.timing.playback = Some(crate::timeline_timing::Playback {
            first: 1,
            end: 3,
            repeat: false,
        });
        d.commit_state(s)?;
        let ranged = Cache::build(
            &d,
            Stamp::of(&d, view, prefs, false)?,
            B,
            &cancel,
            &progress,
        )?;
        check(
            ranged.frames.len() == 2
                && ranged.get(0).is_none()
                && ranged.get(1).is_some()
                && ranged.get(3).is_none(),
            "Playback range precompute was not respected",
        )?;
        let storage =
            crate::storage::UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?;
        base.save(storage.root())?;
        check(
            Preferences::load(storage.root())? == base,
            "Local playback preferences not restored",
        )?;
        let file = storage.root().join("settings/timeline-playback.json");
        std::fs::write(&file, b"damaged").map_err(|e| e.to_string())?;
        check(
            Preferences::load(storage.root()).is_err()
                && std::fs::read(&file).unwrap() == b"damaged",
            "Damaged preferences overwritten",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
#[path = "timeline_playback_tests.rs"]
mod tests;
