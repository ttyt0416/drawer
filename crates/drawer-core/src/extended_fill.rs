//! Independent target predicates and enclosed component selection for fill.
use crate::{
    bucket::{self, Settings as Fill},
    document::Document,
    scene::Point,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Boundary {
    #[default]
    Canvas,
    AnimationFrame,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    #[default]
    MatchSeed,
    AllColors,
    Transparent,
    OtherThanTransparent,
    SurroundedByTransparent,
    Black,
    OtherThanBlack,
    SurroundedByBlack,
    WhiteAndTransparent,
    OtherThanWhiteAndTransparent,
    SurroundedByWhiteAndTransparent,
    AllEnclosedExceptTransparency,
    AllEnclosedIncludingTransparency,
}
impl Target {
    pub const ALL: [Self; 13] = [
        Self::MatchSeed,
        Self::AllColors,
        Self::Transparent,
        Self::OtherThanTransparent,
        Self::SurroundedByTransparent,
        Self::Black,
        Self::OtherThanBlack,
        Self::SurroundedByBlack,
        Self::WhiteAndTransparent,
        Self::OtherThanWhiteAndTransparent,
        Self::SurroundedByWhiteAndTransparent,
        Self::AllEnclosedExceptTransparency,
        Self::AllEnclosedIncludingTransparency,
    ];
    pub fn enclosed(self) -> bool {
        matches!(
            self,
            Self::SurroundedByTransparent
                | Self::SurroundedByBlack
                | Self::SurroundedByWhiteAndTransparent
                | Self::AllEnclosedExceptTransparency
                | Self::AllEnclosedIncludingTransparency
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub target: Target,
    pub semi_transparent: bool,
    pub connected: bool,
    pub boundary: Boundary,
    pub vector_centerline: bool,
    pub excluded_color: Option<[u8; 4]>,
    pub excluded_tolerance: u8,
    pub include_outlines: bool,
    pub outline_distance: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            target: Target::MatchSeed,
            semi_transparent: false,
            connected: true,
            boundary: Boundary::Canvas,
            vector_centerline: false,
            excluded_color: None,
            excluded_tolerance: 0,
            include_outlines: false,
            outline_distance: 1,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (1..=32).contains(&self.outline_distance)
    }
    pub fn transparent(self, p: [u8; 4], t: f64) -> bool {
        p[3] as f64 / 255.0 <= t || self.semi_transparent && p[3] < 255
    }
    pub fn eligible(self, p: [u8; 4], origin: [u8; 4], s: Fill) -> bool {
        let transparent = self.transparent(p, s.alpha_tolerance);
        let black = !transparent && (0..3).all(|i| p[i] as f64 / 255.0 <= s.color_tolerance);
        let white_transparent =
            transparent || (0..3).all(|i| (255 - p[i]) as f64 / 255.0 <= s.color_tolerance);
        match self.target {
            Target::MatchSeed => {
                if self.transparent(origin, s.alpha_tolerance) {
                    transparent
                } else {
                    !transparent
                        && (0..3)
                            .all(|i| p[i].abs_diff(origin[i]) as f64 / 255.0 <= s.color_tolerance)
                }
            }
            Target::AllColors => true,
            Target::Transparent => transparent,
            Target::OtherThanTransparent | Target::SurroundedByTransparent => !transparent,
            Target::Black => black,
            Target::OtherThanBlack
            | Target::SurroundedByBlack
            | Target::AllEnclosedExceptTransparency
            | Target::AllEnclosedIncludingTransparency => !black,
            Target::WhiteAndTransparent => white_transparent,
            Target::OtherThanWhiteAndTransparent | Target::SurroundedByWhiteAndTransparent => {
                !white_transparent
            }
        }
    }
    pub fn writable(self, p: [u8; 4], alpha_tolerance: f64) -> bool {
        !(self.target == Target::AllEnclosedExceptTransparency
            && self.transparent(p, alpha_tolerance)
            || p[3] > 0
                && self.excluded_color.is_some_and(|c| {
                    (0..3).all(|i| p[i].abs_diff(c[i]) <= self.excluded_tolerance)
                }))
    }
}
pub fn frame(
    doc: &Document,
    s: Settings,
) -> Result<Option<crate::animation_output::Frame>, String> {
    if s.boundary == Boundary::Canvas {
        return Ok(None);
    }
    let timeline = doc
        .animation
        .as_ref()
        .ok_or("Animation boundary requires an animation")?;
    if doc.current_frame >= timeline.frames {
        return Err("Active animation frame is outside the timeline".into());
    }
    doc.scene
        .animation_output
        .at(doc.current_frame)
        .map(Some)
        .ok_or("Set an animation output frame first".into())
}
pub fn inside(doc: &Document, frame: Option<crate::animation_output::Frame>, p: Point) -> bool {
    frame.is_none_or(|f| f.contains(doc.scene.transform.inverse().map(p)))
}
pub(crate) fn enclosed(
    closed: &mut [u8],
    domain: &[u8],
    w: usize,
    h: usize,
    start: usize,
    connected: bool,
) -> Result<Vec<u32>, String> {
    let mut accepted = vec![];
    let mut component = Vec::<u32>::new();
    component
        .try_reserve_exact(closed.len())
        .map_err(|_| "Enclosed fill queue allocation failed")?;
    for seed in 0..closed.len() {
        if closed[seed] != 1 {
            continue;
        }
        component.clear();
        component.push(seed as u32);
        closed[seed] = 2;
        let mut head = 0;
        let mut exterior = false;
        let mut has_seed = false;
        while head < component.len() {
            let i = component[head] as usize;
            head += 1;
            has_seed |= i == start;
            let x = i % w;
            let y = i / w;
            exterior |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            for q in bucket::neighbors(i, w, h).into_iter().flatten() {
                if domain[q] == 0 {
                    exterior = true;
                }
                if closed[q] == 1 {
                    closed[q] = 2;
                    component.push(q as u32);
                }
            }
        }
        if !exterior && (!connected || has_seed) {
            accepted.extend_from_slice(&component);
        }
    }
    Ok(accepted)
}
