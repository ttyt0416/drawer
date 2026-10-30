//! Graph edits keep source geometry untouched and commit one document transaction.
use crate::{
    document::Document,
    keyframes::{Curve, Owner, Pose, Track},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    #[default]
    OffsetX,
    OffsetY,
    ScaleX,
    ScaleY,
    Rotation,
    PivotX,
    PivotY,
    Opacity,
}
impl Channel {
    pub const ALL: [Self; 8] = [
        Self::OffsetX,
        Self::OffsetY,
        Self::ScaleX,
        Self::ScaleY,
        Self::Rotation,
        Self::PivotX,
        Self::PivotY,
        Self::Opacity,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::OffsetX => "Offset X",
            Self::OffsetY => "Offset Y",
            Self::ScaleX => "Scale X",
            Self::ScaleY => "Scale Y",
            Self::Rotation => "Rotation",
            Self::PivotX => "Pivot X",
            Self::PivotY => "Pivot Y",
            Self::Opacity => "Opacity",
        }
    }
    pub fn get(self, p: Pose) -> f64 {
        match self {
            Self::OffsetX => p.offset[0],
            Self::OffsetY => p.offset[1],
            Self::ScaleX => p.scale[0],
            Self::ScaleY => p.scale[1],
            Self::Rotation => p.degrees,
            Self::PivotX => p.pivot[0],
            Self::PivotY => p.pivot[1],
            Self::Opacity => p.opacity,
        }
    }
    fn set(self, p: &mut Pose, v: f64) -> Result<(), String> {
        match self {
            Self::OffsetX => p.offset[0] = v,
            Self::OffsetY => p.offset[1] = v,
            Self::ScaleX => p.scale[0] = v,
            Self::ScaleY => p.scale[1] = v,
            Self::Rotation => p.degrees = v,
            Self::PivotX => p.pivot[0] = v,
            Self::PivotY => p.pivot[1] = v,
            Self::Opacity => p.opacity = v,
        }
        if p.valid() {
            Ok(())
        } else {
            Err("Value exceeds this pose channel's limits".into())
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    #[default]
    Free,
    Time,
    Value,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub time: [f64; 2],
    pub value: [f64; 2],
    pub channel: Channel,
    pub axis: Axis,
    pub snap_time: bool,
    pub time_step: u32,
    pub snap_value: bool,
    pub value_step: f64,
    pub split_handles: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            time: [0., 23.],
            value: [-10., 110.],
            channel: Channel::OffsetX,
            axis: Axis::Free,
            snap_time: true,
            time_step: 1,
            snap_value: false,
            value_step: 1.,
            split_handles: false,
        }
    }
}
impl Preferences {
    pub fn valid(self) -> bool {
        self.version == 1
            && self.time.iter().all(|v| v.is_finite() && v.abs() <= 20000.)
            && self.time[1] - self.time[0] >= 0.01
            && self.time[1] - self.time[0] <= 20000.
            && self.value.iter().all(|v| v.is_finite() && v.abs() <= 2e9)
            && self.value[1] - self.value[0] >= 1e-6
            && (1..=10000).contains(&self.time_step)
            && self.value_step.is_finite()
            && (1e-9..=1e9).contains(&self.value_step)
    }
    pub fn snap(self, old: [f64; 2], mut p: [f64; 2]) -> Result<[f64; 2], String> {
        if !self.valid() || !p.iter().chain(&old).all(|v| v.is_finite()) {
            return Err("Invalid graph coordinates/options".into());
        }
        if self.axis == Axis::Value {
            p[0] = old[0];
        } else if self.snap_time {
            p[0] = (p[0] / self.time_step as f64).round() * self.time_step as f64;
        }
        if self.axis == Axis::Time {
            p[1] = old[1];
        } else if self.snap_value {
            p[1] = (p[1] / self.value_step).round() * self.value_step;
        }
        Ok(p)
    }
    pub fn zoom(&mut self, factors: [f64; 2], center: [f64; 2]) -> Result<(), String> {
        if !factors
            .iter()
            .all(|f| f.is_finite() && (0.01..=100.).contains(f))
            || !center.iter().all(|v| v.is_finite())
        {
            return Err("Invalid graph zoom".into());
        }
        let mut p = *self;
        for (i, range) in [&mut p.time, &mut p.value].into_iter().enumerate() {
            for v in range {
                *v = center[i] + (*v - center[i]) / factors[i];
            }
        }
        if !p.valid() {
            return Err("Graph zoom exceeds view limits".into());
        }
        *self = p;
        Ok(())
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local graph settings".into());
        }
        let path = root.join("settings/animation-graph.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let b = crate::book_io::read_local(&path, 8192)?;
        let p: Self = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid graph view settings; original preserved".into());
        }
        Ok(p)
    }
    pub fn save(self, root: &Path) -> Result<(), String> {
        use std::io::Write;
        if !self.valid() || !crate::font_library::local(root) {
            return Err("Invalid local graph view settings".into());
        }
        let parent = root.join("settings");
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        let mut stage = crate::animation_export::Stage::new(&parent)?;
        let mut f = stage.create("graph.json")?;
        f.write_all(&serde_json::to_vec_pretty(&self).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(
            stage.directory.join("graph.json"),
            parent.join("animation-graph.json"),
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Incoming,
    Outgoing,
}
#[derive(Clone, Debug)]
pub enum Edit {
    Move {
        from: u32,
        point: [f64; 2],
        options: Preferences,
        replace: bool,
    },
    Handle {
        frame: u32,
        side: Handle,
        point: [f64; 2],
        options: Preferences,
    },
    Curve {
        frame: u32,
        curve: Curve,
    },
    Scale {
        first: u32,
        last: u32,
        time: [u32; 2],
        pivot: u32,
        value: f64,
        value_pivot: f64,
        channel: Channel,
        replace: bool,
    },
    TimelineLabel {
        id: u64,
        frame: u32,
    },
    TrackLabel {
        id: u64,
        first: u32,
    },
}
pub struct Prepared {
    state: DocumentState,
    id: u64,
    revision: u64,
    pub summary: String,
}
impl Prepared {
    pub fn matches(&self, d: &Document) -> bool {
        self.id == d.id && self.revision == d.revision
    }
    pub fn track(&self, owner: Owner) -> Option<&Track> {
        self.state.scene.keyframes.get(owner)
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if !self.matches(d) {
            return Err("Graph changed; prepare the edit again".into());
        }
        d.commit_state(self.state)
    }
}
fn points(curve: Curve) -> [[f64; 2]; 2] {
    match curve {
        Curve::Bezier { first, second } => [first, second],
        _ => [[1. / 3.; 2], [2. / 3.; 2]],
    }
}
pub fn handle_point(t: &Track, frame: u32, side: Handle, ch: Channel) -> Option<[f64; 2]> {
    let i = t.keys.iter().position(|k| k.frame == frame)?;
    let (a, b, n) = match side {
        Handle::Outgoing => (t.keys.get(i)?, t.keys.get(i + 1)?, 0),
        Handle::Incoming => (t.keys.get(i.checked_sub(1)?)?, t.keys.get(i)?, 1),
    };
    let p = points(a.curve)[n];
    Some([
        a.frame as f64 + (b.frame - a.frame) as f64 * p[0],
        ch.get(a.pose) + (ch.get(b.pose) - ch.get(a.pose)) * p[1],
    ])
}
fn set_handle(t: &mut Track, i: usize, n: usize, p: [f64; 2], ch: Channel) -> Result<(), String> {
    let a = t.keys[i];
    let b = t.keys[i + 1];
    let mut handles = points(a.curve);
    let dy = ch.get(b.pose) - ch.get(a.pose);
    let y = if dy.abs() < 1e-12 {
        if (p[1] - ch.get(a.pose)).abs() > 1e-9 {
            return Err("A flat segment only accepts handles at the same value".into());
        }
        handles[n][1]
    } else {
        (p[1] - ch.get(a.pose)) / dy
    };
    handles[n] = [(p[0] - a.frame as f64) / (b.frame - a.frame) as f64, y];
    let curve = Curve::Bezier {
        first: handles[0],
        second: handles[1],
    };
    if !curve.valid() {
        return Err("Keep handles inside their segment and in time order; split handles for independent editing".into());
    }
    t.keys[i].curve = curve;
    Ok(())
}
pub fn prepare(d: &Document, owner: Owner, edit: Edit, budget: usize) -> Result<Prepared, String> {
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Graph edit exceeds available working memory".into());
    }
    let frames = d.animation.as_ref().ok_or("Choose an animation")?.frames;
    let mut s = d.snapshot();
    let summary = match edit {
        Edit::TimelineLabel { id, frame } => {
            if frame >= frames {
                return Err("Label outside timeline".into());
            }
            let l = s
                .scene
                .cels
                .labels
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Choose a timeline label")?;
            let old = l.frame;
            l.frame = frame;
            format!(
                "Move timeline label '{}' {} → {}",
                l.name,
                old + 1,
                frame + 1
            )
        }
        Edit::TrackLabel { id, first } => {
            let l = s
                .scene
                .timing
                .edit
                .labels
                .iter()
                .find(|l| l.id == id)
                .ok_or("Choose a track range label")?;
            if let Some(clip) = l
                .clip
                .and_then(|id| s.scene.timing.clips.iter().find(|c| c.id == id))
            {
                let n = s
                    .scene
                    .catalog
                    .tracks
                    .index(clip.track, d.animation.as_ref().unwrap().tracks.len())
                    .ok_or("Label track removed")?;
                crate::cels::editable(d, crate::layer_properties::Target::Track(n as u16))?;
            }
            let l = s
                .scene
                .timing
                .edit
                .labels
                .iter_mut()
                .find(|l| l.id == id)
                .unwrap();
            let end = first
                .checked_add(l.end - l.first)
                .filter(|e| *e <= frames)
                .ok_or("Range label outside timeline")?;
            let old = l.first;
            l.first = first;
            l.end = end;
            format!(
                "Move track range label '{}' {} → {}",
                l.name,
                old + 1,
                first + 1
            )
        }
        other => {
            crate::keyframes::writable(d, owner)?;
            let t = s
                .scene
                .keyframes
                .tracks
                .iter_mut()
                .find(|t| t.owner == owner)
                .ok_or("Enable this owner's keyframes first")?;
            if !t.enabled {
                return Err("Enable keyframes before graph editing".into());
            }
            match other {
                Edit::Move {
                    from,
                    point,
                    options,
                    replace,
                } => {
                    let i = t
                        .keys
                        .iter()
                        .position(|k| k.frame == from)
                        .ok_or("Choose a key")?;
                    let mut k = t.keys[i];
                    let p = options.snap([from as f64, options.channel.get(k.pose)], point)?;
                    let time = p[0].round();
                    if time < 0. || time >= frames as f64 {
                        return Err("Key outside timeline".into());
                    }
                    k.frame = time as u32;
                    options.channel.set(&mut k.pose, p[1])?;
                    if k.frame != from && t.keys.iter().any(|old| old.frame == k.frame) && !replace
                    {
                        return Err("Key time collision; explicitly select replacement".into());
                    }
                    t.keys
                        .retain(|old| old.frame != from && old.frame != k.frame);
                    t.keys.push(k);
                    t.keys.sort_by_key(|k| k.frame);
                    format!(
                        "Move {} key {} → {}",
                        options.channel.name(),
                        from + 1,
                        k.frame + 1
                    )
                }
                Edit::Handle {
                    frame,
                    side,
                    point,
                    options,
                } => {
                    let i = t
                        .keys
                        .iter()
                        .position(|k| k.frame == frame)
                        .ok_or("Choose a key")?;
                    let old = handle_point(t, frame, side, options.channel)
                        .ok_or("This endpoint has no handle on that side")?;
                    let p = options.snap(old, point)?;
                    let center = [frame as f64, options.channel.get(t.keys[i].pose)];
                    let (segment, n) = match side {
                        Handle::Outgoing => {
                            if i + 1 >= t.keys.len() {
                                return Err("No next key".into());
                            }
                            (i, 0)
                        }
                        Handle::Incoming => (i.checked_sub(1).ok_or("No previous key")?, 1),
                    };
                    set_handle(t, segment, n, p, options.channel)?;
                    if !options.split_handles {
                        let mirror = [2. * center[0] - p[0], 2. * center[1] - p[1]];
                        match side {
                            Handle::Outgoing if i > 0 => {
                                set_handle(t, i - 1, 1, mirror, options.channel)?
                            }
                            Handle::Incoming if i + 1 < t.keys.len() => {
                                set_handle(t, i, 0, mirror, options.channel)?
                            }
                            _ => {}
                        }
                    }
                    format!(
                        "Edit {:?} handle at {}; {}",
                        side,
                        frame + 1,
                        if options.split_handles {
                            "independent tangent"
                        } else {
                            "joined adjacent tangent"
                        }
                    )
                }
                Edit::Curve { frame, curve } => {
                    if !curve.valid() {
                        return Err("Invalid curve handles".into());
                    }
                    t.keys
                        .iter_mut()
                        .find(|k| k.frame == frame)
                        .ok_or("Choose a key")?
                        .curve = curve;
                    "Change segment interpolation".into()
                }
                Edit::Scale {
                    first,
                    last,
                    time,
                    pivot,
                    value,
                    value_pivot,
                    channel,
                    replace,
                } => {
                    if first > last
                        || last >= frames
                        || pivot >= frames
                        || time.iter().any(|n| !(1..=1000).contains(n))
                        || !value.is_finite()
                        || value.abs() > 1000.
                        || !value_pivot.is_finite()
                    {
                        return Err("Invalid key scale range/factor/pivot".into());
                    }
                    let selected = t
                        .keys
                        .iter()
                        .copied()
                        .filter(|k| (first..=last).contains(&k.frame))
                        .collect::<Vec<_>>();
                    if selected.is_empty() {
                        return Err("No keys in the selected range".into());
                    }
                    let mut keys = t
                        .keys
                        .iter()
                        .filter(|k| !(first..=last).contains(&k.frame))
                        .map(|k| (k.frame, *k))
                        .collect::<BTreeMap<_, _>>();
                    for mut k in selected {
                        let delta = k.frame as i64 - pivot as i64;
                        let scaled = (delta.unsigned_abs() * time[0] as u64 + time[1] as u64 / 2)
                            / time[1] as u64;
                        let frame = pivot as i64
                            + if delta < 0 {
                                -(scaled as i64)
                            } else {
                                scaled as i64
                            };
                        if !(0..frames as i64).contains(&frame) {
                            return Err("Scaled key outside timeline".into());
                        }
                        k.frame = frame as u32;
                        let v = value_pivot + (channel.get(k.pose) - value_pivot) * value;
                        channel.set(&mut k.pose, v)?;
                        if keys.contains_key(&k.frame) && !replace {
                            return Err("Scaled keys collide; explicitly select replacement (later selected key wins)".into());
                        }
                        keys.insert(k.frame, k);
                    }
                    t.keys = keys.into_values().collect();
                    format!(
                        "Scale selected keys in time by {}/{} and {} values by {value}",
                        time[0],
                        time[1],
                        channel.name()
                    )
                }
                _ => unreachable!(),
            }
        }
    };
    if !s.scene.keyframes.valid() {
        return Err("Invalid graph key/curve state".into());
    }
    crate::layer_groups::finish(d, &s)?;
    Ok(Prepared {
        state: s,
        id: d.id,
        revision: d.revision,
        summary,
    })
}

pub fn sample(t: &Track, ch: Channel, time: f64) -> Option<f64> {
    if !time.is_finite() {
        return None;
    }
    if !t.enabled || t.keys.is_empty() {
        return Some(ch.get(t.hold));
    }
    let i = t.keys.partition_point(|k| (k.frame as f64) <= time);
    if i == 0 {
        return Some(ch.get(t.keys[0].pose));
    }
    let a = t.keys[i - 1];
    if i == t.keys.len() {
        return Some(ch.get(a.pose));
    }
    let b = t.keys[i];
    Some(
        ch.get(a.pose)
            + (ch.get(b.pose) - ch.get(a.pose))
                * a.curve
                    .weight((time - a.frame as f64) / (b.frame - a.frame) as f64),
    )
}
#[cfg(test)]
#[path = "graph_edit_tests.rs"]
mod tests;
