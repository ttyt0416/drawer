//! Page-owned keyed animation output frames, following preserved page geometry.
use crate::{
    document::Document,
    scene::{Affine, Point, Rect, Scene},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub frame: u32,
    pub bounds: Rect,
    pub creation: Affine,
}
impl Frame {
    fn valid(self) -> bool {
        self.frame < crate::animation::MAX_FRAMES
            && self.bounds.valid()
            && (0..2).all(|i| {
                self.bounds.max[i] > self.bounds.min[i]
                    && self.bounds.min[i].abs() <= 1e9
                    && self.bounds.max[i].abs() <= 1e9
            })
            && self.creation.valid()
    }
    pub fn contains(self, original: Point) -> bool {
        self.bounds.contains(self.creation.map(original))
    }
    pub fn corners(self, transform: Affine) -> [Point; 4] {
        [
            self.bounds.min,
            [self.bounds.max[0], self.bounds.min[1]],
            self.bounds.max,
            [self.bounds.min[0], self.bounds.max[1]],
        ]
        .map(|p| transform.map(self.creation.inverse().map(p)))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    #[serde(
        default,
        skip_serializing_if = "crate::animation_finish::Settings::is_default"
    )]
    pub settings: crate::animation_finish::Settings,
    pub keys: Vec<Frame>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.keys.is_empty() && self.settings.is_default()
    }
    pub fn valid(&self) -> bool {
        self.settings.valid()
            && self.keys.len() <= crate::animation::MAX_FRAMES as usize
            && self.keys.iter().all(|f| f.valid())
            && self.keys.windows(2).all(|k| k[0].frame < k[1].frame)
    }
    pub fn at(&self, frame: u32) -> Option<Frame> {
        let n = self.keys.partition_point(|k| k.frame <= frame);
        n.checked_sub(1).map(|i| self.keys[i])
    }
    pub fn bytes(&self) -> usize {
        self.keys.len() * std::mem::size_of::<Frame>()
            + if self.settings.is_default() { 0 } else { 32 }
    }
}
pub fn prepare(doc: &Document, bounds: Rect) -> Result<Scene, String> {
    let animation = doc
        .animation
        .as_ref()
        .ok_or("Create an animation before setting its output frame")?;
    if doc.current_frame >= animation.frames {
        return Err("Active animation frame is outside the timeline".into());
    }
    let key = Frame {
        frame: doc.current_frame,
        bounds,
        creation: doc.scene.transform,
    };
    if !key.valid() {
        return Err("Output frame needs finite nonzero bounds".into());
    }
    let mut scene = doc.scene.clone();
    let keys = &mut scene.animation_output.keys;
    match keys.binary_search_by_key(&key.frame, |k| k.frame) {
        Ok(i) => keys[i] = key,
        Err(i) => keys.insert(i, key),
    }
    if !scene.valid()
        || serde_json::to_vec(&scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
    {
        return Err("Output frame exceeds page geometry budget".into());
    }
    Ok(scene)
}

pub fn prepare_state(
    doc: &Document,
    bounds: Rect,
    frames: u32,
) -> Result<crate::sizing::DocumentState, String> {
    let mut state = doc.snapshot();
    if doc.animation.is_some() {
        state.scene = prepare(doc, bounds)?;
        return Ok(state);
    }
    let timeline = crate::animation::Timeline {
        fps: Default::default(),
        frames,
        tracks: vec![],
        audio: vec![],
    };
    let key = Frame {
        frame: doc.current_frame,
        bounds,
        creation: doc.scene.transform,
    };
    if !timeline.valid() || doc.current_frame >= frames || !key.valid() {
        return Err("Invalid local animation length/output frame".into());
    }
    state.animation = Some(timeline);
    state.scene.animation_output.keys = vec![key];
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Output frame exceeds page geometry budget".into());
    }
    Ok(state)
}
