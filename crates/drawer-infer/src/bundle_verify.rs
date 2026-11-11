use super::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "drawer-model-cache-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn all_five_local_models_install_run_register_move_and_forget_without_absolute_registration_paths()
{
    let t = Temp::new();
    let store = UserStorage::at(t.0.join("user")).unwrap();
    let input = fixtures::input();
    let hints = fixtures::hints();
    for task in Task::ALL {
        let source = fixtures::bundle(&t.0.join(format!("source-{task:?}")), task).unwrap();
        let inspected = review(&source, task).unwrap();
        let r = install(
            &store,
            &source,
            task,
            &inspected.manifest_sha256,
            true,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(
            registered(&store, task).unwrap().unwrap().manifest_sha256,
            inspected.manifest_sha256
        );
        exact_cache(r.path(&store).parent().unwrap()).unwrap();
        run_bound(
            &r.path(&store),
            task,
            &input,
            if task == Task::HintColor {
                Some(&hints)
            } else {
                None
            },
            Some(&r.manifest_sha256),
        )
        .unwrap()
        .validate()
        .unwrap();
    }
    let registry = fs::read(registry_path(&store)).unwrap();
    assert!(
        !String::from_utf8(registry.clone())
            .unwrap()
            .contains(&store.root().display().to_string())
    );
    let moved = t.0.join("moved");
    fs::rename(store.root(), &moved).unwrap();
    let moved = UserStorage::at(moved).unwrap();
    assert_eq!(fs::read(registry_path(&moved)).unwrap(), registry);
    let r = registered(&moved, Task::Body).unwrap().unwrap();
    assert!(r.path(&moved).is_file());
    run_bound(
        &r.path(&moved),
        Task::Body,
        &input,
        None,
        Some(&r.manifest_sha256),
    )
    .unwrap();
    forget(&moved, Task::Body).unwrap();
    assert!(registered(&moved, Task::Body).unwrap().is_none());
    assert!(r.path(&moved).is_file());
}
#[test]
fn consent_disk_changed_review_corrupt_cache_and_unknown_entries_never_replace_registration() {
    let t = Temp::new();
    let store = UserStorage::at(t.0.join("user")).unwrap();
    let source = fixtures::bundle(&t.0.join("source"), Task::Hand).unwrap();
    let inspected = review(&source, Task::Hand).unwrap();
    assert!(
        install(
            &store,
            &source,
            Task::Hand,
            &inspected.manifest_sha256,
            false,
            u64::MAX
        )
        .is_err()
    );
    assert!(
        install(
            &store,
            &source,
            Task::Hand,
            &inspected.manifest_sha256,
            true,
            0
        )
        .is_err()
    );
    assert!(!store.root().join("models").exists());
    assert!(!registry_path(&store).exists());
    let original = fs::read(&source).unwrap();
    let mut m: serde_json::Value = serde_json::from_slice(&original).unwrap();
    m["version"] = "changed".into();
    fs::write(&source, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(
        install(
            &store,
            &source,
            Task::Hand,
            &inspected.manifest_sha256,
            true,
            u64::MAX
        )
        .is_err()
    );
    assert!(!registry_path(&store).exists());
    fs::write(&source, original).unwrap();
    let r = install(
        &store,
        &source,
        Task::Hand,
        &inspected.manifest_sha256,
        true,
        u64::MAX,
    )
    .unwrap();
    let before = fs::read(registry_path(&store)).unwrap();
    let cache = r.path(&store);
    fs::write(cache.parent().unwrap().join("weights.onnx"), b"corrupt").unwrap();
    assert!(
        install(
            &store,
            &source,
            Task::Hand,
            &inspected.manifest_sha256,
            true,
            u64::MAX
        )
        .is_err()
    );
    assert_eq!(
        fs::read(cache.parent().unwrap().join("weights.onnx")).unwrap(),
        b"corrupt"
    );
    assert_eq!(fs::read(registry_path(&store)).unwrap(), before);
    fs::write(cache.parent().unwrap().join("user-note.txt"), b"keep").unwrap();
    assert!(
        install(
            &store,
            &source,
            Task::Hand,
            &inspected.manifest_sha256,
            true,
            u64::MAX
        )
        .is_err()
    );
    assert_eq!(
        fs::read(cache.parent().unwrap().join("user-note.txt")).unwrap(),
        b"keep"
    );
}
#[test]
fn unknown_license_invalid_onnx_approval_pin_and_bad_registry_are_refused() {
    let t = Temp::new();
    let store = UserStorage::at(t.0.join("user")).unwrap();
    let source = fixtures::bundle(&t.0.join("source"), Task::Body).unwrap();
    let original = fs::read(&source).unwrap();
    let mut m: serde_json::Value = serde_json::from_slice(&original).unwrap();
    m["license"] = "unknown".into();
    fs::write(&source, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(review(&source, Task::Body).unwrap_err().contains("license"));
    fs::write(&source, &original).unwrap();
    assert!(
        run_bound(
            &source,
            Task::Body,
            &fixtures::input(),
            None,
            Some(&"0".repeat(64))
        )
        .unwrap_err()
        .contains("changed")
    );
    let weights = source.parent().unwrap().join("weights.onnx");
    fs::write(&weights, b"not ONNX").unwrap();
    let mut m: serde_json::Value = serde_json::from_slice(&original).unwrap();
    m["weights_sha256"] = hash(b"not ONNX").into();
    fs::write(&source, serde_json::to_vec(&m).unwrap()).unwrap();
    let corrupt_pin = hash(&fs::read(&source).unwrap());
    assert!(
        run_bound(
            &source,
            Task::Body,
            &fixtures::input(),
            None,
            Some(&corrupt_pin)
        )
        .is_err()
    );
    fs::write(registry_path(&store), b"{broken").unwrap();
    let before = fs::read(registry_path(&store)).unwrap();
    assert!(registered(&store, Task::Body).is_err());
    assert!(forget(&store, Task::Body).is_err());
    assert_eq!(fs::read(registry_path(&store)).unwrap(), before);
}
#[test]
fn unapproved_or_nonlocal_helper_inputs_do_not_create_scratch_or_spawn() {
    let t = Temp::new();
    let source = fixtures::bundle(&t.0.join("source"), Task::Body).unwrap();
    let scratch = t.0.join("scratch");
    let mut request = Request {
        manifest: source.display().to_string(),
        task: Task::Body,
        manifest_sha256: String::new(),
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let exe = std::env::current_exe().unwrap();
    assert!(
        execute(
            &scratch,
            &exe,
            &request,
            &fixtures::input(),
            None,
            &cancel,
            std::time::Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(!scratch.exists());
    request.manifest_sha256 = hash(&fs::read(&source).unwrap());
    assert!(
        execute(
            &scratch,
            Path::new(r"\\drawer.invalid\share\helper.exe"),
            &request,
            &fixtures::input(),
            None,
            &cancel,
            std::time::Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(!scratch.exists());
    request.manifest = "https://drawer.invalid/manifest.json".into();
    assert!(
        execute(
            &scratch,
            &exe,
            &request,
            &fixtures::input(),
            None,
            &cancel,
            std::time::Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(!scratch.exists());
}
