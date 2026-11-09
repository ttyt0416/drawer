use crate::{color_tables::Settings, palette_exchange::*};
use std::path::Path;
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn fixture() -> Settings {
    let mut s = Settings::default();
    s.add([200, 10, 30, 128]).unwrap();
    s.add([10, 20, 30, 0]).unwrap();
    s
}
pub fn policies() -> Result<(), String> {
    let current = fixture();
    let b = Bundle::capture(&current, false)?;
    let copy = b.prepare(&current, Policy::Copy)?;
    copy.check(&current, false)?;
    check(
        copy.after.tables[0] == current.tables[0]
            && copy.after.tables[1].name != "Local colors"
            && copy.after.tables[1].id != 1
            && copy.after.tables[1]
                .colors
                .iter()
                .zip(&current.tables[0].colors)
                .all(|(a, b)| a.id != b.id && a.color == b.color),
        "Copy overwrote IDs/names/colors",
    )?;
    let replace = b.prepare(&current, Policy::ReplaceNames)?;
    check(
        replace.check(&current, false).is_err()
            && replace.check(&current, true).is_ok()
            && replace.after.tables.len() == 1
            && replace.after.tables[0].id == current.tables[0].id,
        "Palette replacement bypassed confirmation / target ID",
    )?;
    let mut changed = current.clone();
    changed.add([1; 4])?;
    check(
        replace.check(&changed, true).is_err(),
        "Stale palette import accepted",
    )?;
    let mut full = current.clone();
    for i in 1..16 {
        full.create(&format!("Table{i}"))?;
    }
    check(
        b.prepare(&full, Policy::Copy).is_err() && full.tables.len() == 16,
        "Palette capacity mutated current settings",
    )?;
    let mut ambiguous = current.clone();
    ambiguous.create("Local colors")?;
    check(
        b.prepare(&ambiguous, Policy::ReplaceNames).is_err(),
        "Ambiguous replacement name accepted",
    )?;
    Ok(())
}
pub fn files(root: &Path) -> Result<(), String> {
    let b = Bundle::capture(&fixture(), false)?;
    let p = root.join("palette.json");
    b.write(&p, false)?;
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    check(Bundle::read(&p)? == b, "Palette file round trip differs")?;
    check(
        b.write(&p, false).is_err() && std::fs::read(&p).map_err(|e| e.to_string())? == bytes,
        "Palette export replaced without confirmation",
    )?;
    let temp = root.join("palette.json.palette-write.tmp");
    std::fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
    check(
        b.write(&p, true).is_err()
            && std::fs::read(&temp).map_err(|e| e.to_string())? == b"foreign",
        "Palette export removed a foreign temporary",
    )?;
    std::fs::remove_file(temp).map_err(|e| e.to_string())?;
    b.write(&p, true)?;
    for kind in 0..6 {
        let mut v = serde_json::to_value(&b).unwrap();
        match kind {
            0 => v["version"] = serde_json::json!(2),
            1 => v["tables"][0]["colors"][0]["id"] = v["tables"][0]["id"].clone(),
            2 => v["tables"][0]["colors"][0]["color"] = serde_json::json!([256, 0, 0, 0]),
            3 => v["tables"][0]["layout"]["size"] = serde_json::json!(0),
            4 => v["tables"][0]["name"] = serde_json::json!("bad\nname"),
            _ => v["unknown"] = serde_json::json!(true),
        }
        check(
            Bundle::decode(&serde_json::to_vec(&v).unwrap()).is_err(),
            "Malformed palette accepted",
        )?;
    }
    check(
        Bundle::decode(&vec![b' '; 2 * 1024 * 1024 + 1]).is_err() && Bundle::decode(b"{").is_err(),
        "Large / truncated palette accepted",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("palette-exchange");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        policies()?;
        files(&dir)
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn remapped_ids_copy_names_confirm_replace_capacity_and_stale() {
        super::policies().unwrap();
    }
    #[test]
    fn local_files_damage_unknown_duplicate_ids_and_foreign_temp_preserved() {
        let t = crate::test_support::TempDir::new();
        super::files(&t.0).unwrap();
    }
}
