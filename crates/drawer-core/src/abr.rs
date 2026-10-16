//! Read-only inspection of a deliberately bounded ABR subset; no vendor engine settings.
use crate::{
    assets::BrushTip,
    brush_presets::Preset,
    material_library::{Kind, Library},
};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};
const LIMIT: usize = 16 * 1024 * 1024;
const MASK_BUDGET: usize = 2 * 1024 * 1024;
fn invalid(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.into())
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub values: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub number: usize,
    pub kind: u16,
    pub name: String,
    pub notes: Vec<String>,
    pub sample: Option<Sample>,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub version: u16,
    pub entries: Vec<Entry>,
    pub notices: Vec<String>,
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if count > self.0.len() {
            return Err(invalid("Truncated ABR field or record"));
        }
        let (part, tail) = self.0.split_at(count);
        self.0 = tail;
        Ok(part)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn long(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn coord(&mut self) -> io::Result<i64> {
        Ok(self.long()? as i32 as i64)
    }
}
impl Report {
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut f = fs::File::open(path)?;
        if !f.metadata()?.is_file() || f.metadata()?.len() > LIMIT as u64 {
            return Err(invalid("ABR input exceeds 16 MiB"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut f)
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::inspect(&bytes)
    }
    pub fn inspect(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > LIMIT {
            return Err(invalid("ABR input exceeds 16 MiB"));
        }
        let mut r = Reader(bytes);
        let version = r.short()?;
        let mut report = Self { version, entries: vec![], notices: vec![
            "Fixture-verified subset only; vendor-file compatibility is not qualified.".into(),
            "Tip density only. Spacing, antialiasing, pressure, scatter, color and other dynamics are not converted. Drawer defaults are used.".into(),
        ] };
        if !matches!(version, 1 | 2) {
            report.notices.push(format!(
                "ABR version {version} is unsupported. No records converted; original preserved."
            ));
            return Ok(report);
        }
        let count = r.short()? as usize;
        if !(1..=256).contains(&count) {
            return Err(invalid("ABR record count must be 1–256"));
        }
        let mut used = 0;
        for number in 1..=count {
            let kind = r.short()?;
            let length = r.long()? as usize;
            let data = r.take(length)?;
            let mut entry = Entry {
                number,
                kind,
                name: format!("ABR tip {number}"),
                notes: vec![],
                sample: None,
            };
            if kind != 2 {
                entry.notes.push(if kind == 1 {
                    "Computed brush: unsupported".into()
                } else {
                    format!("Record type {kind}: unsupported")
                });
            } else {
                match sampled(data, version, &mut entry, MASK_BUDGET - used) {
                    Ok(sample) => {
                        used += sample.values.len();
                        entry.sample = Some(sample);
                    }
                    Err(e) => entry.notes.push(e.to_string()),
                }
            }
            report.entries.push(entry);
        }
        if !r.0.is_empty() {
            return Err(invalid(
                "Unexpected bytes after ABR record table; unverified variant",
            ));
        }
        Ok(report)
    }
    /// Return staged values only. Caller publishes the material, then updates its settings draft.
    pub fn convert(
        &self,
        index: usize,
        name: &str,
        library: &Library,
    ) -> io::Result<(Library, Preset)> {
        let sample = self
            .entries
            .get(index)
            .and_then(|e| e.sample.as_ref())
            .ok_or_else(|| invalid("Select a convertible sampled tip"))?;
        let side = sample.width.max(sample.height);
        if !(1..=512).contains(&side)
            || sample.values.len() != sample.width as usize * sample.height as usize
        {
            return Err(invalid("Invalid decoded mask extent"));
        }
        // Transparent square padding keeps the imported aspect ratio in the tip engine.
        let mut values = vec![0; side as usize * side as usize];
        let left = (side - sample.width) / 2;
        let top = (side - sample.height) / 2;
        for y in 0..sample.height as usize {
            let start = (top as usize + y) * side as usize + left as usize;
            values[start..start + sample.width as usize].copy_from_slice(
                &sample.values[y * sample.width as usize..(y + 1) * sample.width as usize],
            );
        }
        let next = library.register_mask(name, Kind::Tip, side, side, values)?;
        let id = next.assets().last().unwrap().id;
        let snapshot = crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        let mut preset = Preset::capture(&snapshot, BrushTip::Pen, name).map_err(invalid)?;
        preset.values[0] = sample.width.max(sample.height) as f64;
        preset.sequence = crate::tip_sequence::Settings {
            enabled: true,
            count: 1,
            tips: [id, 0, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        };
        preset.validate().map_err(invalid)?;
        Ok((next, preset))
    }
}
fn sampled(data: &[u8], version: u16, entry: &mut Entry, remaining: usize) -> io::Result<Sample> {
    let mut r = Reader(data);
    let misc = r.long()?;
    let spacing = r.short()?;
    if version == 2 {
        let units = r.long()? as usize;
        if units > 256 {
            return Err(invalid("Name exceeds 256 UTF-16 units"));
        }
        let text = r
            .take(units * 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect::<Vec<_>>();
        let name = String::from_utf16(&text).map_err(|_| invalid("Invalid UTF-16 brush name"))?;
        let name = name.trim_end_matches('\0').trim();
        if name.chars().any(char::is_control) {
            return Err(invalid("Control character in brush name"));
        }
        if !name.is_empty() {
            entry.name = name.chars().take(64).collect();
        }
    }
    let aa = r.byte()?;
    r.take(8)?; // Legacy short bounds; the long bounds determine the mask extent.
    let [top, left, bottom, right] = [r.coord()?, r.coord()?, r.coord()?, r.coord()?];
    let depth = r.short()?;
    let compression = r.byte()?;
    let (width, height) = (right - left, bottom - top);
    entry.notes.push(format!("{width}×{height}; {depth}-bit; compression {compression}; spacing {spacing}, AA {aa}, flags {misc:#x} retained only in this report"));
    if !(1..=512).contains(&width) || !(1..=512).contains(&height) {
        return Err(invalid(
            "Unsupported/invalid extent: 1–512 pixels per side required",
        ));
    }
    if depth != 8 {
        return Err(invalid(format!(
            "Depth {depth}: unsupported; only 8-bit density fixtures verified"
        )));
    }
    if compression != 0 {
        return Err(invalid(format!(
            "Compression {compression}: unsupported; only raw tips verified"
        )));
    }
    let size = (width * height) as usize;
    if size > remaining {
        return Err(invalid("Inspection mask budget exceeded (2 MiB)"));
    }
    if r.0.len() != size {
        return Err(invalid(
            "Raw density size mismatch / unverified trailing attributes",
        ));
    }
    let values = r.take(size)?.to_vec();
    if values.iter().all(|v| *v == 0) {
        return Err(invalid("Empty density mask"));
    }
    Ok(Sample {
        width: width as u32,
        height: height as u32,
        values,
    })
}
/// Independently generated wire-format fixture. No original application brush data.
fn fixture(version: u16) -> Vec<u8> {
    let mut payload = vec![];
    payload.extend(0u32.to_be_bytes());
    payload.extend(25u16.to_be_bytes());
    if version == 2 {
        let name = "Own diagonal\0".encode_utf16().collect::<Vec<_>>();
        payload.extend((name.len() as u32).to_be_bytes());
        for ch in name {
            payload.extend(ch.to_be_bytes());
        }
    }
    payload.push(1);
    payload.extend([0; 8]);
    for coord in [0i32, 0, 2, 2] {
        payload.extend(coord.to_be_bytes());
    }
    payload.extend(8u16.to_be_bytes());
    payload.push(0);
    payload.extend([255, 0, 0, 128]);
    let mut out = version.to_be_bytes().to_vec();
    out.extend(2u16.to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend(3u32.to_be_bytes());
    out.extend([4, 5, 6]);
    out.extend(2u16.to_be_bytes());
    out.extend((payload.len() as u32).to_be_bytes());
    out.extend(payload);
    out
}
/// Used by unit tests and the installed app's hidden smoke run.
pub fn verify(root: &Path) -> io::Result<(Library, Preset)> {
    let path = root.join("independent-brush.abr");
    let original = fixture(2);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    f.write_all(&original)?;
    drop(f);
    let report = Report::read(&path)?;
    if report.entries.len() != 2
        || report.entries[0].sample.is_some()
        || report.entries[1]
            .sample
            .as_ref()
            .map(|s| s.values.as_slice())
            != Some(&[255, 0, 0, 128])
    {
        return Err(invalid(
            "ABR inspection/unsupported record separation failed",
        ));
    }
    let (library, preset) = report.convert(1, "ABR diagonal", &Library::default())?;
    preset.save(&root.join("converted-brush.json"), false)?;
    library.save(&root.join("converted-materials.json"))?;
    if fs::read(path)? != original
        || Preset::read(&root.join("converted-brush.json"))? != preset
        || Library::read(&root.join("converted-materials.json"))? != library
    {
        return Err(invalid("ABR original/preset/mask persistence failed"));
    }
    Ok((library, preset))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_versions_masks_names_and_presets() {
        for v in [1, 2] {
            let r = Report::inspect(&fixture(v)).unwrap();
            assert_eq!(r.version, v);
            assert!(r.entries[0].notes[0].contains("Computed"));
            let (lib, p) = r.convert(1, "Brush", &Library::default()).unwrap();
            assert_eq!(lib.assets()[0].values, [255, 0, 0, 128]);
            assert!(p.sequence.enabled);
            assert_eq!(p.values[0], 2.0);
            assert_eq!(p.dynamics, Default::default());
            assert_eq!(
                p.values[6],
                crate::basic_brush::value(
                    crate::tool_properties::Tool::Pen,
                    crate::tool_properties::Property::Spacing
                )
            );
            assert!(r.convert(0, "No", &lib).is_err());
            assert!(r.convert(1, "brush", &lib).is_err());
            assert_eq!(lib.assets().len(), 1);
        }
        assert_eq!(
            Report::inspect(&fixture(2)).unwrap().entries[1].name,
            "Own diagonal"
        );
    }
    #[test]
    fn square_padding_preserves_rectangular_tip_and_invalid_names_do_not_publish() {
        let mut r = Report::inspect(&fixture(1)).unwrap();
        r.entries[1].sample = Some(Sample {
            width: 1,
            height: 2,
            values: vec![255, 128],
        });
        let empty = Library::default();
        for name in ["", "Na\nme", "Name\tcontrol"] {
            assert!(r.convert(1, name, &empty).is_err());
        }
        let (lib, p) = r.convert(1, "Tall", &empty).unwrap();
        assert_eq!(lib.assets()[0].values, [255, 0, 128, 0]);
        assert_eq!((lib.assets()[0].width, lib.assets()[0].height), (2, 2));
        assert_eq!(p.values[0], 2.0);
        assert!(empty.assets().is_empty());
        let mut broken = fixture(2);
        broken[29..31].copy_from_slice(&0xd800u16.to_be_bytes());
        assert!(
            Report::inspect(&broken).unwrap().entries[1]
                .sample
                .is_none()
        );
        for count in [0u16, 257] {
            let mut b = fixture(1);
            b[2..4].copy_from_slice(&count.to_be_bytes());
            assert!(Report::inspect(&b).is_err());
        }
    }
    #[test]
    fn framing_and_lengths_never_yield_partial_conversion() {
        let bytes = fixture(2);
        for n in 0..bytes.len() {
            assert!(Report::inspect(&bytes[..n]).is_err(), "truncation {n}");
        }
        let mut bad_count = bytes.clone();
        bad_count[2..4].copy_from_slice(&65535u16.to_be_bytes());
        assert!(Report::inspect(&bad_count).is_err());
        let mut unknown = bytes.clone();
        unknown[13..15].copy_from_slice(&65535u16.to_be_bytes());
        assert!(
            Report::inspect(&unknown).unwrap().entries[1]
                .sample
                .is_none()
        );
        let mut b = bytes.clone();
        b[15..19].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Report::inspect(&b).is_err());
        let mut b = bytes;
        b.push(0);
        assert!(Report::inspect(&b).is_err());
        assert!(Report::inspect(&vec![0; LIMIT + 1]).is_err());
    }
    #[test]
    fn unsupported_depth_compression_extents_attributes_names_and_budget() {
        // First record takes nine bytes; sampled payload starts at byte nineteen.
        let base = fixture(1);
        for (offset, data, note) in [
            (19 + 31, vec![0, 16], "Depth"),
            (19 + 33, vec![1], "Compression"),
            (19 + 23, (-1i32).to_be_bytes().to_vec(), "extent"),
            (19 + 19, 513i32.to_be_bytes().to_vec(), "extent"),
        ] {
            let mut b = base.clone();
            b[offset..offset + data.len()].copy_from_slice(&data);
            let r = Report::inspect(&b).unwrap();
            assert!(r.entries[1].sample.is_none(), "{offset}");
            assert!(
                r.entries[1].notes.iter().any(|n| n.contains(note)),
                "{offset}"
            );
        }
        let mut b = base.clone();
        b[15..19].copy_from_slice(&39u32.to_be_bytes());
        b.push(1);
        assert!(Report::inspect(&b).unwrap().entries[1].sample.is_none());
        let mut b = fixture(2);
        b[25..29].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Report::inspect(&b).unwrap().entries[1].sample.is_none());
        let mut e = Entry {
            number: 1,
            kind: 2,
            name: String::new(),
            notes: vec![],
            sample: None,
        };
        assert!(sampled(&base[19..], 1, &mut e, 3).is_err());
        let mut b = base;
        b[53..57].fill(0);
        assert!(Report::inspect(&b).unwrap().entries[1].sample.is_none());
        for v in [0u16, 3, 6, 10, 65535] {
            let r = Report::inspect(&v.to_be_bytes()).unwrap();
            assert!(r.entries.is_empty());
            assert!(r.notices.last().unwrap().contains("unsupported"));
            assert!(r.convert(0, "No", &Library::default()).is_err());
        }
    }
}
