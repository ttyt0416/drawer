use crate::{
    animation::{Cell, Exposure, Fps, Timeline, Track},
    cels::{self, Label, Link},
    document::{Background, CanvasSpec, Document},
    layer_properties::Target,
    layer_search::Key,
    sizing::DocumentState,
};
use std::collections::{BTreeMap, BTreeSet};
pub enum Edit {
    Enable,
    NewTrack(String),
    RenameTrack {
        track: u64,
        name: String,
    },
    DeleteTrack(u64),
    CopyTrack {
        track: u64,
        independent: bool,
    },
    NewCel {
        track: u64,
        name: String,
    },
    Duplicate {
        track: u64,
        source: u64,
        independent: bool,
    },
    Expose {
        track: u64,
        source: u64,
        frame: u32,
    },
    RemoveExposure {
        track: u64,
        frame: u32,
    },
    MoveSlot {
        from: u64,
        cell: u64,
        to: u64,
    },
    DeleteCel(u64),
    RenameCel {
        id: u64,
        name: String,
    },
    Paste {
        track: u64,
        replace: Option<u64>,
        document: Box<Document>,
    },
    Label {
        name: String,
        frame: u32,
    },
    DeleteLabel(u64),
}
pub struct Prepared {
    pub state: DocumentState,
    pub track: Option<u64>,
    pub source: Option<u64>,
    pub affected: usize,
    pub summary: String,
}
pub fn create(d: &Document, frames: u32, fps: Fps) -> Result<DocumentState, String> {
    if d.animation.is_some() {
        return Err("This document already has a timeline".into());
    }
    let mut s = d.snapshot();
    s.animation = Some(Timeline {
        fps,
        frames,
        tracks: vec![],
        audio: vec![],
    });
    if !s.animation.as_ref().unwrap().valid() {
        return Err("Use1..10000 frames and a valid rational FPS".into());
    }
    Ok(s)
}
pub fn uses(d: &Document, source: u64) -> Vec<(u64, u32)> {
    let mut rows = vec![];
    if let Some(a) = &d.animation {
        for (n, t) in a.tracks.iter().enumerate() {
            let track = d.scene.catalog.tracks.id(n);
            for e in &t.exposures {
                let cell = d.scene.catalog.cell_id(track, e.cell);
                if d.scene.cels.source(track, cell) == Some(source) {
                    rows.push((track, e.frame));
                }
            }
        }
    }
    rows
}
pub fn reference_ranges(d: &Document, source: u64) -> String {
    let mut rows = vec![];
    if let Some(a) = &d.animation {
        for (n, t) in a.tracks.iter().enumerate() {
            let id = d.scene.catalog.tracks.id(n);
            for (i, e) in t.exposures.iter().enumerate() {
                if d.scene.cels.source(id, d.scene.catalog.cell_id(id, e.cell)) == Some(source) {
                    let end = t.exposures.get(i + 1).map_or(t.end, |next| next.frame);
                    rows.push(format!(
                        "{} [track{id}] frames{}–{}",
                        t.name,
                        e.frame + 1,
                        end
                    ));
                }
            }
        }
    }
    if rows.is_empty() {
        "No exposure references (unused definition)".into()
    } else {
        rows.join("; ")
    }
}
fn track(s: &DocumentState, id: u64) -> Result<usize, String> {
    let a = s.animation.as_ref().ok_or("Missing timeline")?;
    s.scene
        .catalog
        .tracks
        .index(id, a.tracks.len())
        .ok_or_else(|| "Track removed".into())
}
fn current(s: &DocumentState, frame: u32) -> Result<(), String> {
    if s.animation.as_ref().is_none_or(|a| frame >= a.frames) {
        Err("Frame outside the timeline".into())
    } else {
        Ok(())
    }
}
fn blank(d: &Document, name: &str, budget: usize) -> Result<Document, String> {
    let mut child = Document::create_with_color(
        CanvasSpec {
            width: d.width,
            height: d.height,
            dpi: d.dpi,
            background: Background::Transparent,
        },
        budget,
        d.color.clone(),
    )
    .map_err(|e| e.to_string())?;
    child.title = name.into();
    Ok(child)
}
fn attach(
    s: &mut DocumentState,
    track_id: u64,
    source: u64,
    frame: u32,
    budget: usize,
) -> Result<(), String> {
    current(s, frame)?;
    let t = track(s, track_id)?;
    let existing = s
        .scene
        .cels
        .links
        .iter()
        .find(|l| l.track == track_id && l.source == source)
        .and_then(|l| {
            s.scene
                .catalog
                .cells
                .get(&track_id)?
                .index(l.cell, s.animation.as_ref()?.tracks[t].cells.len())
        });
    let cell = if let Some(c) = existing {
        c
    } else {
        let part = s.scene.cels.get(source).ok_or("Cel definition missing")?;
        let child = part.document().ok_or("Cel source missing")?;
        let pixels = cels::pixels(child, budget)?;
        let name = part.name.clone();
        let size = [child.width, child.height];
        let a = s.animation.as_mut().unwrap();
        if a.tracks.iter().map(|t| t.cells.len()).sum::<usize>() >= 10000 {
            return Err("Cel slot limit10000".into());
        }
        let c = a.tracks[t].cells.len();
        a.tracks[t].cells.push(Cell {
            name,
            width: size[0],
            height: size[1],
            offset: [0, 0],
            pixels,
        });
        let ids = s.scene.catalog.cells.entry(track_id).or_default();
        ids.append(c + 1)?;
        let id = ids.id(c);
        s.scene.cels.links.push(Link {
            track: track_id,
            cell: id,
            source,
        });
        c
    };
    let t = &mut s.animation.as_mut().unwrap().tracks[t];
    match t.exposures.binary_search_by_key(&frame, |e| e.frame) {
        Ok(n) => t.exposures[n].cell = cell,
        Err(n) => t.exposures.insert(n, Exposure { frame, cell }),
    }
    t.end = t.end.max(frame + 1);
    Ok(())
}
fn duplicate(s: &mut DocumentState, source: u64, budget: usize) -> Result<u64, String> {
    let p = s.scene.cels.get(source).ok_or("Cel definition missing")?;
    let child = p.open()?;
    let name = format!("{} copy", p.name.chars().take(59).collect::<String>());
    cels::insert(s, &child, &name, budget)
}
fn add_track(s: &mut DocumentState, name: String) -> Result<u64, String> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("Track name limit256 bytes".into());
    }
    let a = s.animation.as_mut().ok_or("Missing timeline")?;
    if a.tracks.len() >= 32 {
        return Err("Track limit32".into());
    }
    a.tracks.push(Track {
        name,
        cells: vec![],
        exposures: vec![],
        end: a.frames,
    });
    s.scene.catalog.tracks.append(a.tracks.len())?;
    let id = s.scene.catalog.tracks.id(a.tracks.len() - 1);
    s.scene.catalog.cells.entry(id).or_default();
    Ok(id)
}
fn working(s: &DocumentState, frame: u32) -> Result<Document, String> {
    crate::layer_merge::preview(s, frame)
}
pub fn prepare(d: &Document, edit: Edit, budget: usize) -> Result<Prepared, String> {
    let mut s = cels::enable(d, budget)?;
    current(&s, d.current_frame)?;
    let mut selected = None;
    let mut source = None;
    let mut affected = 0;
    let summary = match edit {
        Edit::Enable => {
            "Enable editable cel definitions; existing pixels and exposures remain".into()
        }
        Edit::NewTrack(name) => {
            let id = add_track(&mut s, name.clone())?;
            let child = blank(d, "Blank cel", budget)?;
            let cel = cels::insert(&mut s, &child, "Blank cel", budget)?;
            attach(&mut s, id, cel, d.current_frame, budget)?;
            selected = Some(id);
            source = Some(cel);
            "Add track and assign a new blank cel at the current frame".into()
        }
        Edit::RenameTrack { track: id, name } => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            if name.trim().is_empty() || name.len() > 256 {
                return Err("Invalid track name".into());
            }
            s.animation.as_mut().unwrap().tracks[n].name = name;
            selected = Some(id);
            "Rename track".into()
        }
        Edit::NewCel { track: id, name } => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            let child = blank(d, &name, budget)?;
            let cel = cels::insert(&mut s, &child, &name, budget)?;
            attach(&mut s, id, cel, d.current_frame, budget)?;
            selected = Some(id);
            source = Some(cel);
            "Add a blank cel layer and assign the current frame".into()
        }
        Edit::Duplicate {
            track: id,
            source: cel,
            independent,
        } => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            let cel = if independent {
                duplicate(&mut s, cel, budget)?
            } else {
                cel
            };
            attach(&mut s, id, cel, d.current_frame, budget)?;
            selected = Some(id);
            source = Some(cel);
            if independent {
                "Duplicate editable content with a new cel ID"
            } else {
                "Share the same cel ID; edits affect all its exposures"
            }
            .into()
        }
        Edit::Expose {
            track: id,
            source: cel,
            frame,
        } => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            attach(&mut s, id, cel, frame, budget)?;
            selected = Some(id);
            source = Some(cel);
            format!(
                "Assign cel{cel} at frame{}; existing assignment at that frame is replaced",
                frame + 1
            )
        }
        Edit::RemoveExposure { track: id, frame } => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            let t = &mut s.animation.as_mut().unwrap().tracks[n];
            let pos = t
                .exposures
                .binary_search_by_key(&frame, |e| e.frame)
                .map_err(|_| "Select an exposure start")?;
            t.exposures.remove(pos);
            selected = Some(id);
            "Remove exposure marker only; cel definitions and unused slots remain".into()
        }
        Edit::RenameCel { id, name } => {
            if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err("Invalid cel name".into());
            }
            let p = s
                .scene
                .cels
                .definitions
                .iter_mut()
                .find(|p| p.object == id)
                .ok_or("Cel removed")?;
            p.name = name.clone();
            for l in &s.scene.cels.links {
                if l.source == id {
                    let n = track(&s, l.track)?;
                    let c = s.scene.catalog.cells[&l.track]
                        .index(l.cell, s.animation.as_ref().unwrap().tracks[n].cells.len())
                        .unwrap();
                    cels::editable(
                        d,
                        Target::Cell {
                            track: n as u16,
                            cell: c as u16,
                        },
                    )?;
                    s.animation.as_mut().unwrap().tracks[n].cells[c].name = name.clone();
                }
            }
            source = Some(id);
            "Rename shared cel definition".into()
        }
        Edit::DeleteCel(id) => {
            let w = working(&s, d.current_frame)?;
            let links = s
                .scene
                .cels
                .links
                .iter()
                .filter(|l| l.source == id)
                .cloned()
                .collect::<Vec<_>>();
            affected = uses(&w, id).len();
            let keys = links
                .iter()
                .map(|l| Key::Cell {
                    track: l.track,
                    cell: l.cell,
                })
                .collect::<Vec<_>>();
            for l in &links {
                let (t, c, _) = cels::slot(&w, l.track, l.cell).ok_or("Slot removed")?;
                cels::editable(
                    &w,
                    Target::Cell {
                        track: t as u16,
                        cell: c as u16,
                    },
                )?;
            }
            if !keys.is_empty() {
                s = crate::search_edit::delete_batch(&w, &keys, true)?;
            }
            s.scene.cels.links.retain(|l| l.source != id);
            s.scene.cels.definitions.retain(|p| p.object != id);
            format!(
                "DELETE cel{id} and {affected} exposure starts across all tracks; one Undo restores all references. {}",
                reference_ranges(&w, id)
            )
        }
        Edit::DeleteTrack(id) => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            let w = working(&s, d.current_frame)?;
            let t = &w.animation.as_ref().unwrap().tracks[n];
            affected = t.exposures.len();
            let mut keys = vec![Key::Track(id)];
            keys.extend((0..t.cells.len()).map(|n| Key::Cell {
                track: id,
                cell: w.scene.catalog.cell_id(id, n),
            }));
            s = crate::search_edit::delete_batch(&w, &keys, true)?;
            selected = None;
            format!(
                "Delete track and {affected} exposure starts; unused global cel definitions remain"
            )
        }
        Edit::MoveSlot { from, cell, to } => {
            let w = working(&s, d.current_frame)?;
            let (t, c, _) = cels::slot(&w, from, cell).ok_or("Slot missing")?;
            let target = track(&s, to)?;
            if from == to {
                return Err("Choose another track".into());
            }
            cels::editable(
                &w,
                Target::Cell {
                    track: t as u16,
                    cell: c as u16,
                },
            )?;
            cels::editable(&w, Target::Track(target as u16))?;
            let cel = s
                .scene
                .cels
                .source(from, cell)
                .ok_or("Editable source missing")?;
            if s.scene
                .cels
                .links
                .iter()
                .any(|l| l.track == to && l.source == cel)
            {
                return Err("The destination already has this shared cel; choose another track to preserve each slot's placement".into());
            }
            let old_props = w
                .scene
                .layer_properties
                .cells
                .get(&(((t as u64) << 32) | c as u64))
                .copied();
            let old_alpha = w.scene.clipping.alpha.contains(&Target::Cell {
                track: t as u16,
                cell: c as u16,
            });
            let old_info = w
                .scene
                .catalog
                .cell_info
                .get(&from)
                .and_then(|m| m.get(&cell))
                .cloned();
            let frames = w.animation.as_ref().unwrap().tracks[t]
                .exposures
                .iter()
                .filter(|e| e.cell == c)
                .map(|e| e.frame)
                .collect::<Vec<_>>();
            affected = frames.len();
            let old_offset = w.animation.as_ref().unwrap().tracks[t].cells[c].offset;
            s = crate::search_edit::delete_batch(&w, &[Key::Cell { track: from, cell }], true)?;
            for f in &frames {
                attach(&mut s, to, cel, *f, budget)?;
            }
            if frames.is_empty() {
                attach(&mut s, to, cel, d.current_frame, budget)?;
                let n = track(&s, to)?;
                s.animation.as_mut().unwrap().tracks[n]
                    .exposures
                    .retain(|e| e.frame != d.current_frame);
            }
            let n = track(&s, to)?;
            let link = s
                .scene
                .cels
                .links
                .iter()
                .find(|l| l.track == to && l.source == cel)
                .unwrap();
            let c = s.scene.catalog.cells[&to]
                .index(
                    link.cell,
                    s.animation.as_ref().unwrap().tracks[n].cells.len(),
                )
                .unwrap();
            s.animation.as_mut().unwrap().tracks[n].cells[c].offset = old_offset;
            let new_cell = link.cell;
            if let Some(props) = old_props {
                s.scene
                    .layer_properties
                    .cells
                    .insert(((n as u64) << 32) | c as u64, props);
            }
            if old_alpha {
                s.scene.clipping.alpha.insert(Target::Cell {
                    track: n as u16,
                    cell: c as u16,
                });
            }
            if let Some(info) = old_info {
                s.scene
                    .catalog
                    .cell_info
                    .entry(to)
                    .or_default()
                    .insert(new_cell, info);
            }

            selected = Some(to);
            source = Some(cel);
            format!(
                "Move cel slot and {affected} exposures to another track; target collisions are replaced"
            )
        }
        Edit::CopyTrack {
            track: id,
            independent,
        } => {
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            let w = working(&s, d.current_frame)?;
            let copied = crate::layer_move::copy::prepare(&w, &[Key::Track(id)])?;
            let Key::Track(to) = copied.keys[0] else {
                return Err("Copied track identity missing".into());
            };
            s = copied.state;
            if independent {
                let mut sources = BTreeMap::new();
                let links = s
                    .scene
                    .cels
                    .links
                    .iter()
                    .filter(|l| l.track == to)
                    .cloned()
                    .collect::<Vec<_>>();
                for l in links {
                    let new = if let Some(new) = sources.get(&l.source) {
                        *new
                    } else {
                        let new = duplicate(&mut s, l.source, budget)?;
                        sources.insert(l.source, new);
                        new
                    };
                    s.scene
                        .cels
                        .links
                        .iter_mut()
                        .find(|link| link.track == to && link.cell == l.cell)
                        .unwrap()
                        .source = new;
                }
            }
            selected = Some(to);
            format!(
                "Copy track with {} cel references",
                if independent { "independent" } else { "shared" }
            )
        }
        Edit::Paste {
            track: id,
            replace,
            document,
        } => {
            if document.animation.is_some() {
                return Err("Copy a static drawing or cell content".into());
            }
            let n = track(&s, id)?;
            cels::editable(d, Target::Track(n as u16))?;
            let cel = if let Some(cel) = replace {
                let w = working(&s, d.current_frame)?;
                s = cels::replace(&w, cel, &document, budget)?;
                cel
            } else {
                cels::insert(&mut s, &document, "Pasted cel", budget)?
            };
            attach(&mut s, id, cel, d.current_frame, budget)?;
            selected = Some(id);
            source = Some(cel);
            "Paste editable drawing content and assign the current frame as one edit".into()
        }
        Edit::Label { name, frame } => {
            current(&s, frame)?;
            if name.trim().is_empty()
                || name.len() > 256
                || name.chars().any(char::is_control)
                || s.scene.cels.labels.len() >= 1024
            {
                return Err("Label name/count limit".into());
            }
            let id = s.scene.cels.next_label;
            s.scene.cels.next_label = id.checked_add(1).ok_or("Label IDs exhausted")?;
            s.scene.cels.labels.push(Label { id, name, frame });
            "Add timeline label".into()
        }
        Edit::DeleteLabel(id) => {
            s.scene.cels.labels.retain(|l| l.id != id);
            "Remove timeline label".into()
        }
    };
    cels::reconcile(d, &mut s)?;
    crate::layer_groups::finish(d, &s)?;
    let check = working(&s, d.current_frame)?;
    if !cels::valid_document(&check) {
        return Err("Invalid cel definitions, slots or dimensions".into());
    }
    Ok(Prepared {
        state: s,
        track: selected,
        source,
        affected,
        summary,
    })
}
#[derive(Clone, Copy)]
pub enum Jump {
    First,
    Last,
    Previous,
    Next,
    Number(u32),
    PreviousCel,
    NextCel,
    PreviousKey,
    NextKey,
    PreviousLabel,
    NextLabel,
}
pub fn navigate(d: &Document, track_id: Option<u64>, jump: Jump) -> Result<u32, String> {
    let a = d.animation.as_ref().ok_or("Missing timeline")?;
    let frame = d.current_frame.min(a.frames - 1);
    let (nxt, positions) = match jump {
        Jump::First => return Ok(0),
        Jump::Last => return Ok(a.frames - 1),
        Jump::Previous => return Ok(frame.saturating_sub(1)),
        Jump::Next => return Ok((frame + 1).min(a.frames - 1)),
        Jump::Number(n) => {
            return if n < a.frames {
                Ok(n)
            } else {
                Err("Frame outside timeline".into())
            };
        }
        Jump::PreviousLabel | Jump::NextLabel => (
            matches!(jump, Jump::NextLabel),
            d.scene
                .cels
                .labels
                .iter()
                .map(|l| l.frame)
                .collect::<BTreeSet<_>>(),
        ),
        Jump::PreviousKey | Jump::NextKey => (
            matches!(jump, Jump::NextKey),
            d.scene
                .animation_output
                .keys
                .iter()
                .filter(|k| k.frame < a.frames)
                .map(|k| k.frame)
                .collect(),
        ),
        Jump::PreviousCel | Jump::NextCel => {
            let id = track_id.ok_or("Select a track")?;
            let n = d
                .scene
                .catalog
                .tracks
                .index(id, a.tracks.len())
                .ok_or("Track missing")?;
            if crate::layer_properties::inherited(d, Target::Track(n as u16)).hidden {
                return Err("Selected track is hidden".into());
            }
            let t = &a.tracks[n];
            let mut starts = BTreeSet::new();
            let mut previous = None;
            for e in &t.exposures {
                let cell = d.scene.catalog.cell_id(id, e.cell);
                let identity = d.scene.cels.source(id, cell).unwrap_or(cell);
                if previous != Some(identity) {
                    starts.insert(e.frame);
                    previous = Some(identity);
                }
            }
            (matches!(jump, Jump::NextCel), starts)
        }
    };
    if nxt {
        positions
            .range((std::ops::Bound::Excluded(frame), std::ops::Bound::Unbounded))
            .next()
            .copied()
    } else {
        positions.range(..frame).next_back().copied()
    }
    .ok_or_else(|| "No navigation target in that direction".into())
}
pub fn playback(
    start: u32,
    seconds: f64,
    fps: Fps,
    frames: u32,
    repeat: bool,
) -> Result<(u32, bool), String> {
    if !fps.valid()
        || frames == 0
        || frames > 10000
        || start >= frames
        || !seconds.is_finite()
        || seconds < 0.
    {
        return Err("Invalid playback state".into());
    }
    let ticks = seconds * fps.numerator as f64 / fps.denominator as f64;
    // Compensate only for rounding at an exact frame boundary (for example 1.001s at30000/1001).
    let advance = (ticks + ticks.abs().max(1.) * f64::EPSILON * 4.)
        .floor()
        .min(u64::MAX as f64) as u64;
    let absolute = (start as u64).saturating_add(advance);
    Ok(if repeat {
        ((absolute % frames as u64) as u32, false)
    } else {
        (
            absolute.min(frames as u64 - 1) as u32,
            absolute >= frames as u64,
        )
    })
}
