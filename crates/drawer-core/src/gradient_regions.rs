//! Four-connected closed regions and vector spans, prepared atomically with gradient coverage.
use crate::{
    bucket,
    document::Document,
    gradient::{Area, Settings, VectorFill},
    line_tools::{Data, Path},
    selection::{Mask, Run},
};
pub(crate) fn prepare(
    doc: &Document,
    active: usize,
    s: &Settings,
    reference: bucket::Settings,
) -> Result<(Vec<Area>, Data), String> {
    let w = doc.width as usize;
    let h = doc.height as usize;
    let count = w * h;
    let sampler = crate::fill_reference::Sampler::new(doc, active, reference)?;
    let mut pixels = Vec::with_capacity(count);
    let mut selection = Vec::with_capacity(count);
    for y in 0..h {
        for x in 0..w {
            pixels.push(sampler.pixel(x as u32, y as u32));
            selection.push(
                doc.scene
                    .selection_coverage([x as f64 + 0.5, y as f64 + 0.5]),
            );
        }
    }
    let eligible = |p: usize| {
        selection[p] > 0
            && s.exclude_color != Some(pixels[p])
            && (pixels[p][3] == 0 || pixels[p][..3].iter().all(|v| *v >= 250))
    };
    let mut labels = vec![0u16; count];
    let mut areas = vec![];
    let mut queue = Vec::<usize>::new();
    for seed in 0..count {
        if labels[seed] != 0 || !eligible(seed) {
            continue;
        }
        queue.clear();
        queue.push(seed);
        labels[seed] = u16::MAX;
        let mut cursor = 0;
        let mut closed = true;
        while cursor < queue.len() {
            let p = queue[cursor];
            cursor += 1;
            let x = p % w;
            let y = p / w;
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                closed = false
            }
            for q in bucket::neighbors(p, w, h).into_iter().flatten() {
                if selection[q] == 0 {
                    closed = false
                }
                if labels[q] == 0 && eligible(q) {
                    labels[q] = u16::MAX;
                    queue.push(q)
                }
            }
        }
        if !closed {
            continue;
        }
        if areas.len() == 256 {
            return Err("Freeform gradient exceeds 256 closed regions".into());
        }
        let mut anchors = [([0.0; 2], f64::INFINITY); 2];
        let id = areas.len() as u16 + 1;
        for &p in &queue {
            labels[p] = id;
            let point = [(p % w) as f64 + 0.5, (p / w) as f64 + 0.5];
            for (a, requested) in anchors.iter_mut().zip([s.start, s.end]) {
                let distance = (point[0] - requested[0]).hypot(point[1] - requested[1]);
                if distance < a.1 {
                    *a = (point, distance)
                }
            }
        }
        areas.push(Area {
            anchors: Some([anchors[0].0, anchors[1].0]),
            paint: Mask::default(),
            black: Mask::default(),
            transparent: Mask::default(),
        });
    }
    // Boundary ownership is deterministic when two regions share the same ink pixel.
    let mut owner = labels.clone();
    let mut kind = vec![0u8; count];
    let mut coverage = selection.clone();
    for p in 0..count {
        if owner[p] > 0 && owner[p] != u16::MAX {
            kind[p] = if pixels[p][3] == 0 { 2 } else { 0 };
            continue;
        }
        owner[p] = 0;
        if selection[p] > 0
            && s.exclude_color != Some(pixels[p])
            && pixels[p][3] > 0
            && pixels[p][..3].iter().all(|v| *v <= 32)
        {
            let adjacent = bucket::neighbors(p, w, h)
                .into_iter()
                .flatten()
                .filter_map(|q| (labels[q] > 0 && labels[q] != u16::MAX).then_some(labels[q]))
                .min();
            if let Some(id) = adjacent {
                owner[p] = id;
                kind[p] = 1;
                coverage[p] = ((coverage[p] as u32 * pixels[p][3] as u32 + 127) / 255) as u8;
            }
        }
    }
    let mut lines = doc.scene.lines.clone();
    let mut work = 0usize;
    if s.vectors != VectorFill::Keep {
        if doc.scene.selection.is_some() || doc.scene.selection_mask.is_some() {
            return Err("Clear the raster selection before replacing editable vector spans".into());
        }
        let refs: Vec<_> = doc
            .scene
            .lines
            .layers
            .iter()
            .filter(|l| !l.hidden)
            .flat_map(|l| &l.paths)
            .flat_map(|p| p.segments())
            .map(|(a, b)| (doc.scene.transform.map(a), doc.scene.transform.map(b)))
            .collect();
        for li in 0..lines.layers.len() {
            let layer = &doc.scene.lines.layers[li];
            if layer.locked || !crate::fill_reference::vector(layer, li, reference) {
                continue;
            }
            let mut changed = vec![];
            for path in &layer.paths {
                let mut contacts = vec![None; areas.len()];
                let mut hit_pixels = vec![];
                for y in 0..h {
                    for x in 0..w {
                        work = work.saturating_add(path.points.len() + 1);
                        if work > 64 * 1024 * 1024 {
                            return Err("Freeform vector work limit".into());
                        }
                        let p = y * w + x;
                        let point = [x as f64 + 0.5, y as f64 + 0.5];
                        if s.exclude_color == Some(pixels[p])
                            || !path.hit(doc.scene.transform.inverse().map(point))
                        {
                            continue;
                        }
                        let id = if owner[p] > 0 {
                            Some(owner[p])
                        } else {
                            bucket::neighbors(p, w, h)
                                .into_iter()
                                .flatten()
                                .filter_map(|q| {
                                    (labels[q] > 0 && labels[q] != u16::MAX).then_some(labels[q])
                                })
                                .min()
                        };
                        if let Some(id) = id {
                            contacts[id as usize - 1].get_or_insert(point);
                            hit_pixels.push((p, id));
                        }
                    }
                }
                if hit_pixels.is_empty() {
                    changed.push(path.clone());
                    continue;
                }
                let mut remain = vec![path.clone()];
                if s.vectors == VectorFill::WholeLine {
                    remain.clear()
                } else {
                    for point in contacts.into_iter().flatten() {
                        let mut next = vec![];
                        for part in remain {
                            match crate::line_tools::cut(
                                &part,
                                doc.scene.transform,
                                point,
                                0.1,
                                &refs,
                                &mut work,
                            )? {
                                None => next.push(part),
                                Some(parts) => {
                                    for points in parts {
                                        let id = lines.id()?;
                                        next.push(Path {
                                            id,
                                            points,
                                            closed: false,
                                            width: part.width,
                                            color: part.color,
                                        });
                                    }
                                }
                            }
                        }
                        remain = next;
                    }
                }
                // Preserve gradient ink on the entire removed span, including the portion
                // outside the closed region when Whole line was explicitly requested.
                let fallback = hit_pixels[0].1;
                let span_cost =
                    path.points.len() + remain.iter().map(|p| p.points.len()).sum::<usize>() + 1;
                for y in 0..h {
                    for x in 0..w {
                        work = work.saturating_add(span_cost);
                        if work > 64 * 1024 * 1024 {
                            return Err("Freeform vector work limit".into());
                        }
                        let p = y * w + x;
                        let point = doc
                            .scene
                            .transform
                            .inverse()
                            .map([x as f64 + 0.5, y as f64 + 0.5]);
                        if s.exclude_color != Some(pixels[p])
                            && path.hit(point)
                            && !remain.iter().any(|r| r.hit(point))
                        {
                            owner[p] = if owner[p] > 0 { owner[p] } else { fallback };
                            kind[p] = 0;
                            coverage[p] = 255;
                        }
                    }
                }
                changed.extend(remain);
            }
            lines.layers[li].paths = changed;
        }
    }
    let mut runs = 0usize;
    for y in 0..h {
        let mut x = 0;
        while x < w {
            let p = y * w + x;
            let id = owner[p];
            let k = kind[p];
            let a = coverage[p];
            let start = x;
            x += 1;
            while x < w
                && owner[y * w + x] == id
                && kind[y * w + x] == k
                && coverage[y * w + x] == a
            {
                x += 1
            }
            if id > 0 && id != u16::MAX && a > 0 {
                let area = &mut areas[id as usize - 1];
                let mask = match k {
                    1 => &mut area.black,
                    2 => &mut area.transparent,
                    _ => &mut area.paint,
                };
                mask.runs.push(Run {
                    x: start as i32,
                    y: y as i32,
                    length: (x - start) as u32,
                    alpha: a,
                });
                runs += 1;
                if runs > 32768 {
                    return Err("Freeform coverage exceeds run limit".into());
                }
            }
        }
    }
    if !lines.valid() {
        return Err("Freeform vector result exceeds path limit".into());
    }
    // Keep all classes, including currently hidden ones, for later policy edits.
    Ok((areas, lines))
}
