//! Independent GPL v2 text and an explicitly defined Drawer CSV schema.
use crate::{
    color_tables::{Layout, Swatch, Table},
    palette_exchange::Bundle,
};
use std::path::Path;
fn text(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes)).map_err(|_| {
        "Palette files must be UTF-8; UTF-16 and legacy encodings are unsupported".into()
    })
}
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
fn bundle(
    name: String,
    colors: Vec<([u8; 4], String)>,
    columns: Option<u8>,
) -> Result<Bundle, String> {
    if colors.len() > 256 {
        return Err("A text palette supports at most256 colors".into());
    }
    let b = Bundle {
        version: 1,
        tables: vec![Table {
            id: 1,
            name,
            layout: Layout {
                columns,
                ..Default::default()
            },
            colors: colors
                .into_iter()
                .enumerate()
                .map(|(i, (color, name))| Swatch {
                    id: i as u64 + 2,
                    color,
                    name,
                })
                .collect(),
        }],
    };
    if b.valid() {
        Ok(b)
    } else {
        Err("Invalid palette name, color name, layout, values or capacity; nothing imported".into())
    }
}
fn gpl(path: &Path, input: &str) -> Result<(Bundle, Vec<String>), String> {
    let mut lines = input.lines();
    if lines.next() != Some("GIMP Palette") {
        return Err("Expected GPL GIMP Palette header".into());
    }
    let mut name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(64)
        .collect::<String>();
    let mut columns = None;
    let mut colors = vec![];
    let mut named = false;
    let mut columned = false;
    let mut notes = vec![];
    for (n, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix("Name:") {
            if named || !colors.is_empty() {
                return Err(format!("Duplicate/misplaced GPL name at line{}", n + 2));
            }
            name = value.trim().into();
            named = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("Columns:") {
            if columned || !colors.is_empty() {
                return Err(format!("Duplicate/misplaced GPL columns at line{}", n + 2));
            }
            let value = value
                .trim()
                .parse::<u8>()
                .map_err(|_| "GPL Columns must be0..255")?;
            columns = if value == 0 || value > 32 {
                None
            } else {
                Some(value)
            };
            if value > 32 {
                notes.push(format!("GPL Columns {value} exceeds local32-column layout; using responsive columns. RGB/name values retained."));
            }
            columned = true;
            continue;
        }
        let mut rest = line;
        let mut rgba = [255; 4];
        for value in rgba.iter_mut().take(3) {
            let at = rest.find(char::is_whitespace).unwrap_or(rest.len());
            *value = rest[..at]
                .parse()
                .map_err(|_| format!("GPL RGB must be integers0..255 at line{}", n + 2))?;
            rest = rest[at..].trim_start();
        }
        let label = if rest.is_empty() {
            format!("RGB {},{},{}", rgba[0], rgba[1], rgba[2])
        } else {
            rest.into()
        };
        colors.push((rgba, label));
        if colors.len() > 256 {
            return Err("GPL exceeds local256-color capacity".into());
        }
    }
    Ok((bundle(name, colors, columns)?, notes))
}
fn fields(line: &str) -> Result<Vec<String>, String> {
    let mut chars = line.chars().peekable();
    let mut result = vec![];
    loop {
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => {
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            value.push('"');
                        } else {
                            break;
                        }
                    }
                    Some(c) => value.push(c),
                    None => {
                        return Err(
                            "Unclosed CSV quote; multiline/control names are unsupported".into(),
                        );
                    }
                }
            }
            if chars.peek().is_some_and(|c| *c != ',') {
                return Err("Unexpected characters after CSV quote".into());
            }
        } else {
            while chars.peek().is_some_and(|c| *c != ',') {
                let c = chars.next().unwrap();
                if c == '"' {
                    return Err("CSV quotes must enclose the entire field".into());
                }
                value.push(c);
            }
        }
        if value.len() > 4096 || result.len() == 6 {
            return Err("CSV field/count limit".into());
        }
        result.push(value);
        if chars.next().is_none() {
            break;
        }
    }
    Ok(result)
}
fn csv(input: &str) -> Result<Bundle, String> {
    let mut lines = input.lines();
    if fields(lines.next().ok_or("Missing CSV header")?)? != ["palette", "r", "g", "b", "a", "name"]
    {
        return Err("Drawer CSV header must be palette,r,g,b,a,name".into());
    }
    let mut name = None;
    let mut colors = vec![];
    for (n, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = fields(line)?;
        if row.len() != 6 {
            return Err(format!("CSV needs6 fields at line{}", n + 2));
        }
        if name.as_ref().is_some_and(|v| v != &row[0]) {
            return Err(
                "One CSV export/import represents one palette; palette names must match".into(),
            );
        }
        name = Some(row[0].clone());
        let mut rgba = [0; 4];
        for (value, s) in rgba.iter_mut().zip(&row[1..5]) {
            *value = s
                .trim()
                .parse()
                .map_err(|_| format!("CSV RGBA must be integers0..255 at line{}", n + 2))?;
        }
        colors.push((rgba, row[5].clone()));
        if colors.len() > 256 {
            return Err("CSV exceeds local256-color capacity".into());
        }
    }
    bundle(
        name.ok_or("CSV must contain at least one color")?,
        colors,
        None,
    )
}
pub fn decode(path: &Path, bytes: &[u8]) -> Result<(Bundle, Vec<String>), String> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Palette exchange exceeds2MiB".into());
    }
    match extension(path).as_str(){"json"=>Bundle::decode(bytes).map(|b|(b,vec![])),"gpl"=>gpl(path,text(bytes)?),"csv"=>csv(text(bytes)?).map(|b|(b,vec![])),_=>Err("Palette exchange supports Drawer JSON, GPL and Drawer CSV; other proprietary formats are unsupported".into())}
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn encode(path: &Path, b: &Bundle) -> Result<Vec<u8>, String> {
    if !b.valid() {
        return Err("Invalid palette bundle".into());
    }
    let ext = extension(path);
    if ext == "json" {
        return serde_json::to_vec_pretty(b).map_err(|e| e.to_string());
    }
    if !["gpl", "csv"].contains(&ext.as_str()) {
        return Err("Choose .json, .gpl or .csv for palette exchange".into());
    }
    if b.tables.len() != 1 {
        return Err("GPL/CSV represents one palette; enable Export selected table only".into());
    }
    let t = &b.tables[0];
    let mut out = String::new();
    if ext == "gpl" {
        if t.colors.iter().any(|c| c.color[3] != 255) {
            return Err(
                "GPL has no alpha channel. Choose CSV/JSON to preserve transparent colors".into(),
            );
        }
        out = format!(
            "GIMP Palette\nName: {}\nColumns: {}\n# sRGB8; alpha is always255\n",
            t.name,
            t.layout.columns.unwrap_or(0)
        );
        for c in &t.colors {
            out.push_str(&format!(
                "{} {} {}\t{}\n",
                c.color[0], c.color[1], c.color[2], c.name
            ));
        }
    } else {
        if t.colors.is_empty() {
            return Err("CSV needs at least one color; use JSON for an empty table".into());
        }
        out.push_str("palette,r,g,b,a,name\n");
        for c in &t.colors {
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                quote(&t.name),
                c.color[0],
                c.color[1],
                c.color[2],
                c.color[3],
                quote(&c.name)
            ));
        }
    }
    Ok(out.into_bytes())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("palette-formats-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let b = bundle(
            "색, \"표\"".into(),
            vec![
                ([10, 20, 30, 255], "A, \"B\"".into()),
                ([2, 120, 240, 128], "투명  색".into()),
            ],
            None,
        )?;
        for ext in ["csv", "json"] {
            let path = dir.join(format!("palette.{ext}"));
            b.write(&path, false)?;
            let loaded = Bundle::read(&path)?;
            if loaded.tables[0].name != b.tables[0].name
                || loaded.tables[0]
                    .colors
                    .iter()
                    .map(|c| (&c.name, c.color))
                    .collect::<Vec<_>>()
                    != b.tables[0]
                        .colors
                        .iter()
                        .map(|c| (&c.name, c.color))
                        .collect::<Vec<_>>()
            {
                return Err("CSV/JSON lost names/RGBA".into());
            }
            let before = std::fs::read(&path).unwrap();
            if b.write(&path, false).is_ok() || std::fs::read(&path).unwrap() != before {
                return Err("Unconfirmed palette replacement changed output".into());
            }
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        let path = dir.join("palette.gpl");
        if b.write(&path, false).is_ok() || path.exists() {
            return Err("GPL silently dropped alpha".into());
        }
        let mut opaque = b.clone();
        opaque.tables[0].colors[1].color[3] = 255;
        opaque.write(&path, false)?;
        if Bundle::read(&path)?.tables[0].colors != opaque.tables[0].colors {
            return Err("GPL RGB/labels changed".into());
        }
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
        let gpl=b"\xef\xbb\xbfGIMP Palette\r\nName: Example\r\nColumns: 255\r\n# note\r\n1 2 3 color\r\n";
        let (loaded, notes) = decode(Path::new("example.gpl"), gpl)?;
        if notes.len() != 1 || loaded.tables[0].colors[0].color != [1, 2, 3, 255] {
            return Err("GPL BOM/CRLF/layout notice".into());
        }
        if decode(Path::new("old.gpl"), b"GIMP Palette\n1 2 3\n")?
            .0
            .tables[0]
            .name
            != "old"
        {
            return Err("Old GPL filename fallback".into());
        }
        for bytes in [
            b"GIMP Palette\n256 2 3 bad\n".as_slice(),
            b"GIMP Palette\n1 -2 3 bad\n",
            b"GIMP Palette\nColumns: 256\n",
            b"\xff\xfeG\0I\0",
            b"GIMP Palette\n1 2\n",
        ] {
            if decode(Path::new("bad.gpl"), bytes).is_ok() {
                return Err("Invalid GPL accepted".into());
            }
        }
        for input in [
            "palette,r,g,b,a,name\np,1,2,3,256,bad\n",
            "palette,r,g,b,a,name\np,1,2,3,4,good\np,1,2,3,no,bad\n",
            "palette,r,g,b,a,name\n\"broken,1,2,3,4,name\n",
            "palette,r,g,b,a,name\np,1,2,3,4,a\nq,1,2,3,4,b\n",
        ] {
            if decode(Path::new("bad.csv"), input.as_bytes()).is_ok() {
                return Err("Invalid/partial CSV accepted".into());
            }
        }
        let (_, s) = decode(
            Path::new("bom.csv"),
            [
                b"\xef\xbb\xbf".as_slice(),
                encode(Path::new("a.csv"), &b)?.as_slice(),
            ]
            .concat()
            .as_slice(),
        )?;
        if !s.is_empty() {
            return Err("Unexpected CSV loss".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn gpl_csv_json_values_alpha_quotes_bom_ranges_partial_failure_and_files() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
