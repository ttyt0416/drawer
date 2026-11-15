//! Refuse to replace future schemas; damaged known schemas retain their existing repair policy.
use serde::{Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read},
    path::Path,
};
const LIMIT: u64 = 32 * 1024 * 1024;
fn json(path: &Path) -> io::Result<Option<serde_json::Value>> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut first = [0u8; 1];
    if f.read(&mut first)? == 0 || !matches!(first[0], b'{' | b'[' | b' ' | b'\n' | b'\r' | b'\t') {
        return Ok(None);
    }
    let mut bytes = Vec::from(first);
    f.take(LIMIT).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other(
            "Existing JSON exceeds compatibility inspection limit; preserved",
        ));
    }
    Ok(serde_json::from_slice(&bytes).ok())
}
fn version(old: &serde_json::Value, next: &serde_json::Value) -> io::Result<()> {
    if let (Some(old), Some(next)) = (
        old.get("version").and_then(|v| v.as_u64()),
        next.get("version").and_then(|v| v.as_u64()),
    ) && old > next
    {
        return Err(io::Error::other(
            "Newer settings version; original preserved. Use its matching app or a separate profile.",
        ));
    }
    Ok(())
}
pub fn guard<T: Serialize + DeserializeOwned>(path: &Path, next: &T) -> io::Result<()> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    crate::file_browser::new_path(&absolute).map_err(io::Error::other)?;
    let Some(old) = json(path)? else {
        return Ok(());
    };
    version(&old, &serde_json::to_value(next).map_err(io::Error::other)?)?;
    if let Err(e) = serde_json::from_value::<T>(old)
        && e.to_string().contains("unknown field")
    {
        return Err(io::Error::other(format!(
            "Unsupported settings field; original preserved: {e}"
        )));
    }
    Ok(())
}
pub(crate) fn candidate(
    path: &Path,
    next: &Path,
    validation_error: Option<&str>,
) -> io::Result<()> {
    if let (Some(old), Some(next)) = (json(path)?, json(next)?) {
        version(&old, &next)?;
        if validation_error.is_some_and(|e| e.contains("unknown field")) {
            return Err(io::Error::other(
                "Unsupported JSON field; original preserved",
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Settings {
        version: u32,
        value: u32,
    }
    #[test]
    fn atomic_json_future_versions_unknown_fields_refuse_without_replacing_original() {
        use std::io::Write;
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("settings.json");
        let temp = root.0.join("candidate.tmp");
        let next = br#"{"version":1,"value":2}"#;
        for old in [
            br#"{"version":999,"value":1}"#.as_slice(),
            br#"{"version":1,"value":1,"futureField":true}"#.as_slice(),
        ] {
            std::fs::write(&path, old).unwrap();
            let result = crate::atomic_file::publish(
                &path,
                &temp,
                |w| w.write_all(next),
                |p| {
                    let s: Settings =
                        serde_json::from_slice(&std::fs::read(p)?).map_err(io::Error::other)?;
                    if s.version != 1 {
                        return Err(io::Error::other("Unsupported settings version"));
                    }
                    Ok(())
                },
                &mut |_| Ok(()),
            );
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), old);
            assert!(!temp.exists());
        }
    }
}
