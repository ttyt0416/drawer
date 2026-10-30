//! Explicit rows and identities; sorting CSV rows never changes track/cel order or bindings.
use crate::timesheet::*;
use std::collections::BTreeMap;
const HEADER: &str = "kind,id,parent,order,name,frame,source,file,x,y,width,height,end,fps_num,fps_den,dpi,frames,sha256,properties,options,revision,version";
fn row() -> Vec<String> {
    vec![String::new(); 22]
}
fn bitmap(kind: &str, i: &Bitmap) -> Vec<String> {
    let mut r = row();
    r[0] = kind.into();
    r[1] = i.id.to_string();
    r[4] = i.name.clone();
    r[7] = i.path.clone();
    r[8] = i.origin[0].to_string();
    r[9] = i.origin[1].to_string();
    r[10] = i.width.to_string();
    r[11] = i.height.to_string();
    r[17] = i.hash.iter().map(|b| format!("{b:02x}")).collect();
    r
}
pub fn encode(s: &Sheet) -> Result<Vec<u8>, String> {
    if !s.valid() {
        return Err("Invalid timesheet CSV".into());
    }
    let mut rows = vec![];
    let mut r = row();
    r[0] = "meta".into();
    r[4] = s.format.clone();
    r[5] = s.captured_frame.to_string();
    r[10] = s.width.to_string();
    r[11] = s.height.to_string();
    r[13] = s.fps.numerator.to_string();
    r[14] = s.fps.denominator.to_string();
    r[15] = s.dpi.to_string();
    r[16] = s.frames.to_string();
    r[19] = serde_json::to_string(&s.options).map_err(|e| e.to_string())?;
    r[20] = s.revision.to_string();
    r[21] = s.version.to_string();
    rows.push(r);
    rows.extend(s.images.iter().map(|i| bitmap("image", i)));
    if let Some(i) = &s.common {
        rows.push(bitmap("common", i));
    }
    for t in &s.tracks {
        let mut r = row();
        r[0] = "track".into();
        r[1] = t.id.to_string();
        r[3] = t.order.to_string();
        r[4] = t.name.clone();
        r[12] = t.end.to_string();
        r[18] = serde_json::to_string(&t.properties).map_err(|e| e.to_string())?;
        rows.push(r);
        for c in &t.cells {
            let mut r = row();
            r[0] = "cell".into();
            r[1] = c.id.to_string();
            r[2] = t.id.to_string();
            r[3] = c.order.to_string();
            r[4] = c.name.clone();
            r[6] = c.source.to_string();
            r[8] = c.offset[0].to_string();
            r[9] = c.offset[1].to_string();
            r[18] = serde_json::to_string(&c.properties).map_err(|e| e.to_string())?;
            rows.push(r);
        }
        for k in &t.exposures {
            let mut r = row();
            r[0] = "exposure".into();
            r[2] = t.id.to_string();
            r[5] = k.frame.to_string();
            r[6] = k.cell.to_string();
            rows.push(r);
        }
    }
    let mut out = String::from(HEADER);
    out.push_str("\r\n");
    for r in rows {
        out.push_str(
            &r.iter()
                .map(|s| crate::shot_exchange::quote(s))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
        if out.len() > LIMIT {
            return Err("Timesheet CSV exceeds4MiB".into());
        }
    }
    Ok(out.into_bytes())
}
fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse()
        .map_err(|_| "Invalid or missing timesheet number".into())
}
fn image(r: &[String]) -> Result<Bitmap, String> {
    if r[17].len() != 64 || !r[17].is_ascii() {
        return Err("Expected SHA256 for each cel image".into());
    }
    let mut hash = [0; 32];
    for (n, b) in hash.iter_mut().enumerate() {
        *b = u8::from_str_radix(&r[17][n * 2..n * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(Bitmap {
        id: number(&r[1])?,
        name: r[4].clone(),
        path: r[7].clone(),
        origin: [number(&r[8])?, number(&r[9])?],
        width: number(&r[10])?,
        height: number(&r[11])?,
        hash,
    })
}
pub fn decode(text: &str) -> Result<Sheet, String> {
    let mut rows = crate::shot_exchange::csv_rows(text, 22, 400001)?;
    if rows.is_empty() || rows.remove(0) != HEADER.split(',').collect::<Vec<_>>() {
        return Err(
            "Only Drawer timesheet v1 CSV is supported; no verified external adapter".into(),
        );
    }
    if rows.iter().any(|r| r.len() != 22) {
        return Err("Timesheet CSV column count".into());
    }
    let metas = rows.iter().filter(|r| r[0] == "meta").collect::<Vec<_>>();
    if metas.len() != 1 {
        return Err("Timesheet requires exactly one meta row".into());
    }
    let m = metas[0];
    let mut sheet = Sheet {
        format: m[4].clone(),
        version: number(&m[21])?,
        width: number(&m[10])?,
        height: number(&m[11])?,
        dpi: number(&m[15])?,
        fps: crate::animation::Fps {
            numerator: number(&m[13])?,
            denominator: number(&m[14])?,
        },
        frames: number(&m[16])?,
        captured_frame: number(&m[5])?,
        revision: number(&m[20])?,
        options: serde_json::from_str(&m[19]).map_err(|e| e.to_string())?,
        images: vec![],
        common: None,
        tracks: vec![],
    };
    let mut tracks = BTreeMap::new();
    for r in rows.iter().filter(|r| r[0] == "track") {
        let id = number(&r[1])?;
        if tracks
            .insert(
                id,
                Lane {
                    id,
                    order: number(&r[3])?,
                    name: r[4].clone(),
                    end: number(&r[12])?,
                    properties: serde_json::from_str(&r[18]).map_err(|e| e.to_string())?,
                    cells: vec![],
                    exposures: vec![],
                },
            )
            .is_some()
        {
            return Err("Duplicate track ID".into());
        }
    }
    for r in &rows {
        match r[0].as_str() {
            "meta" | "track" => {}
            "image" => sheet.images.push(image(r)?),
            "common" => {
                if sheet.common.replace(image(r)?).is_some() {
                    return Err("Duplicate common background".into());
                }
            }
            "cell" => {
                let t = tracks
                    .get_mut(&number::<u64>(&r[2])?)
                    .ok_or("Cell refers to a missing track")?;
                t.cells.push(Slot {
                    id: number(&r[1])?,
                    order: number(&r[3])?,
                    name: r[4].clone(),
                    source: number(&r[6])?,
                    offset: [number(&r[8])?, number(&r[9])?],
                    properties: serde_json::from_str(&r[18]).map_err(|e| e.to_string())?,
                });
            }
            "exposure" => {
                let t = tracks
                    .get_mut(&number::<u64>(&r[2])?)
                    .ok_or("Exposure refers to a missing track")?;
                t.exposures.push(Key {
                    frame: number(&r[5])?,
                    cell: number(&r[6])?,
                });
            }
            _ => return Err("Unknown timesheet CSV row type".into()),
        }
    }
    sheet.tracks = tracks.into_values().collect();
    sheet.normalize();
    if !sheet.valid() {
        return Err("Invalid timesheet IDs, version, order, exposure or local filename".into());
    }
    Ok(sheet)
}
