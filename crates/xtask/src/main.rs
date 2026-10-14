use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn relative_path(text: &str) -> io::Result<PathBuf> {
    if text.contains('\\') || text.contains(':') {
        return Err(invalid("전달 경로에는 / 구분자만 사용해야 합니다"));
    }
    let path = Path::new(text);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || text
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return Err(invalid(format!("허용하지 않는 전달 경로: {text}")));
    }
    let extension = path.extension().and_then(|value| value.to_str());
    if !matches!(
        extension,
        Some("rs" | "md" | "toml" | "txt" | "lock" | "ps1")
    ) {
        return Err(invalid(format!("허용하지 않는 전달 파일 형식: {text}")));
    }
    Ok(path.to_owned())
}

fn plain_file(root: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    let mut path = root.to_owned();
    for part in relative.components() {
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(invalid(format!(
                "링크는 전달할 수 없습니다: {}",
                path.display()
            )));
        }
    }
    if !path.is_file() || !path.canonicalize()?.starts_with(root) {
        return Err(invalid("전달 파일이 구현 폴더 밖에 있습니다"));
    }
    let bytes = fs::read(path)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| invalid("전달 파일은 UTF-8이어야 합니다"))?;
    if text.contains('\0') {
        return Err(invalid("바이너리 데이터는 전달할 수 없습니다"));
    }
    Ok(bytes)
}

fn inputs(root: &Path) -> io::Result<Vec<(PathBuf, Vec<u8>)>> {
    let manifest = fs::read_to_string(root.join("handoff-files.txt"))?;
    let mut files = Vec::new();
    for line in manifest.lines().filter(|line| !line.is_empty()) {
        let path = relative_path(line)?;
        if files.iter().any(|(existing, _)| existing == &path) {
            return Err(invalid("전달 목록에 중복 경로가 있습니다"));
        }
        files.push((path.clone(), plain_file(root, &path)?));
    }
    if !files
        .iter()
        .any(|(path, _)| path == Path::new("handoff-files.txt"))
    {
        return Err(invalid("전달 목록 자체를 포함해야 합니다"));
    }
    if !files
        .iter()
        .any(|(path, _)| path == Path::new("docs/specs/feature-checklist.md"))
    {
        return Err(invalid("기능 명세를 전달 목록에 포함해야 합니다"));
    }
    Ok(files)
}

fn tree_files(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(invalid("전달 묶음에 링크가 있습니다"));
        }
        if kind.is_dir() {
            tree_files(root, &path, files)?;
        } else if kind.is_file() {
            files.push(path);
        } else {
            return Err(invalid("일반 파일이 아닌 전달 항목이 있습니다"));
        }
    }
    Ok(())
}

fn verify_bundle(root: &Path, bundle: &Path) -> io::Result<usize> {
    let expected = inputs(root)?;
    let mut actual = Vec::new();
    tree_files(bundle, Path::new(""), &mut actual)?;
    let mut paths: Vec<_> = expected.iter().map(|(path, _)| path.clone()).collect();
    paths.sort();
    actual.sort();
    if actual != paths {
        return Err(invalid("전달 묶음과 전달 목록의 파일이 다릅니다"));
    }
    for (path, bytes) in &expected {
        if fs::read(bundle.join(path))? != *bytes {
            return Err(invalid(format!(
                "전달 파일이 원문과 다릅니다: {}",
                path.display()
            )));
        }
    }
    Ok(expected.len())
}

fn specification_violations(text: &str) -> Vec<&'static str> {
    let mut violations = Vec::new();
    for token in
        text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
    {
        if ["FUN_", "DAT_", "LAB_"].iter().any(|prefix| {
            token.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() >= 6 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        }) {
            violations.push("디컴파일 식별자");
        }
        if token.strip_prefix("0x").is_some_and(|suffix| {
            suffix.len() >= 9 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            violations.push("원본 주소로 의심되는 긴 16진수");
        }
    }
    for marker in [
        "boost::",
        "Planeswalker::",
        "::vftable",
        "TypeDescriptor",
        "undefined8",
        "undefined4",
        "param_1",
        "C:\\PaintToolSAI",
        "C:\\Program Files\\CELSYS",
        "C:/PaintToolSAI",
        "C:/Program Files/CELSYS",
    ] {
        if text.contains(marker) {
            violations.push("원본 코드·자료구조·경로 표기");
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

fn audit_specifications(root: &Path) -> io::Result<usize> {
    let files = inputs(root)?;
    let mut specifications = 0;
    for (path, bytes) in files {
        if path.starts_with("docs/specs") {
            specifications += 1;
            let text = std::str::from_utf8(&bytes).unwrap();
            let violations = specification_violations(text);
            if !violations.is_empty() {
                return Err(invalid(format!(
                    "{}: {}",
                    path.display(),
                    violations.join(", ")
                )));
            }
        }
    }
    if specifications == 0 {
        return Err(invalid("검토할 구현 명세가 없습니다"));
    }
    Ok(specifications)
}

fn prepare(root: &Path) -> io::Result<usize> {
    let files = inputs(root)?;
    audit_specifications(root)?;
    let staging = root.join(".handoff-staging");
    if staging.exists() {
        return Err(invalid("이전 임시 전달 폴더를 먼저 정리해야 합니다"));
    }
    fs::create_dir(&staging)?;
    let result = (|| {
        for (path, bytes) in &files {
            let destination = staging.join(path);
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::write(destination, bytes)?;
        }
        verify_bundle(root, &staging)?;
        let handoff = root.join("handoff");
        fs::create_dir_all(&handoff)?;
        if handoff.symlink_metadata()?.file_type().is_symlink() {
            return Err(invalid("전달 폴더는 링크일 수 없습니다"));
        }
        let latest = handoff.join("latest");
        let previous = handoff.join("previous");
        if previous.exists() {
            return Err(invalid("이전 전달 묶음의 교체 복구를 먼저 마쳐야 합니다"));
        }
        if latest.exists() {
            if latest.symlink_metadata()?.file_type().is_symlink() {
                return Err(invalid("최신 전달 폴더는 링크일 수 없습니다"));
            }
            fs::rename(&latest, &previous)?;
        }
        if let Err(error) = fs::rename(&staging, &latest) {
            if previous.exists() {
                fs::rename(&previous, &latest)?;
            }
            return Err(error);
        }
        if previous.exists() {
            fs::remove_dir_all(&previous)?;
        }
        Ok(files.len())
    })();
    if staging.exists() {
        fs::remove_dir_all(staging)?;
    }
    result
}

fn run() -> io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let command = env::args().nth(1).unwrap_or_default();
    let count = match command.as_str() {
        "prepare-handoff" => prepare(&root)?,
        "verify-handoff" => verify_bundle(&root, &root.join("handoff/latest"))?,
        "audit-specs" => {
            let count = audit_specifications(&root)?;
            println!("구현 명세 {count}개 검사; 원본 내부 코드·주소·자료구조 흔적 0개");
            return Ok(());
        }
        _ => {
            return Err(invalid(
                "사용법: xtask prepare-handoff | verify-handoff | audit-specs",
            ));
        }
    };
    println!("전달 파일 {count}개 검증; 바이너리·추출 리소스·디컴파일 산출물 전달 0개");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("전달 검사 실패: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = env::temp_dir().join(format!(
                "drawer-handoff-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir_all(root.join("docs/specs")).unwrap();
            fs::write(root.join("docs/specs/feature-checklist.md"), "독립 명세").unwrap();
            fs::write(
                root.join("handoff-files.txt"),
                "handoff-files.txt\ndocs/specs/feature-checklist.md\n",
            )
            .unwrap();
            Self(root.canonicalize().unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn denies_path_escape_and_non_source_formats() {
        for path in [
            "../research.txt",
            "C:/research.txt",
            "/research.txt",
            ".git/config",
            "docs//secret.txt",
            "docs\\secret.txt",
            "source.exe",
            "resource.png",
            "source.dll",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn copies_only_the_manifest_and_detects_tampering() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("decompiled.txt"), "excluded research").unwrap();
        fs::write(fixture.0.join("original.exe"), [0, 255]).unwrap();
        assert_eq!(prepare(&fixture.0).unwrap(), 2);
        let bundle = fixture.0.join("handoff/latest");
        assert_eq!(verify_bundle(&fixture.0, &bundle).unwrap(), 2);
        assert!(!bundle.join("original.exe").exists());
        assert!(!bundle.join("decompiled.txt").exists());
        fs::write(bundle.join("extra.txt"), "unexpected").unwrap();
        assert!(verify_bundle(&fixture.0, &bundle).is_err());
        fs::remove_file(bundle.join("extra.txt")).unwrap();
        fs::write(bundle.join("docs/specs/feature-checklist.md"), "changed").unwrap();
        assert!(verify_bundle(&fixture.0, &bundle).is_err());
    }

    #[test]
    fn invalid_input_preserves_the_previous_bundle() {
        let fixture = Fixture::new();
        prepare(&fixture.0).unwrap();
        fs::write(fixture.0.join("handoff-files.txt"), "../research.txt\n").unwrap();
        assert!(prepare(&fixture.0).is_err());
        assert_eq!(
            fs::read_to_string(
                fixture
                    .0
                    .join("handoff/latest/docs/specs/feature-checklist.md")
            )
            .unwrap(),
            "독립 명세"
        );
        assert!(!fixture.0.join(".handoff-staging").exists());
    }

    #[test]
    fn denies_binary_text_and_duplicate_manifest_entries() {
        let fixture = Fixture::new();
        fs::write(
            fixture.0.join("docs/specs/feature-checklist.md"),
            b"MZ\0binary",
        )
        .unwrap();
        assert!(inputs(&fixture.0).is_err());
        fs::write(fixture.0.join("docs/specs/feature-checklist.md"), "spec").unwrap();
        fs::write(
            fixture.0.join("handoff-files.txt"),
            "handoff-files.txt\nhandoff-files.txt\n",
        )
        .unwrap();
        assert!(inputs(&fixture.0).is_err());
    }

    #[test]
    fn refuses_original_code_and_addresses_in_specs() {
        for text in [
            "FUN_140123456",
            "DAT_145000000",
            "0x140123456",
            "boost::shared_ptr",
            "Planeswalker::PWLayer",
            "param_1",
            "C:\\PaintToolSAI\\sai.exe",
        ] {
            assert!(!specification_violations(text).is_empty(), "{text}");
        }
        assert!(
            specification_violations("새 앱의 색상 #FF00AA, 필압 0~1, DPI 300, 실행 취소 100단계")
                .is_empty()
        );
    }

    #[test]
    fn refuses_contaminated_spec_before_replacing_bundle() {
        let fixture = Fixture::new();
        prepare(&fixture.0).unwrap();
        fs::write(
            fixture.0.join("docs/specs/feature-checklist.md"),
            "FUN_140123456",
        )
        .unwrap();
        assert!(prepare(&fixture.0).is_err());
        assert_eq!(
            fs::read_to_string(
                fixture
                    .0
                    .join("handoff/latest/docs/specs/feature-checklist.md")
            )
            .unwrap(),
            "독립 명세"
        );
        assert!(!fixture.0.join(".handoff-staging").exists());
    }
}
