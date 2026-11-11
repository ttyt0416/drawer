use super::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("drawer-full-backup-{}", nonce()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn state() -> (AtomicBool, AtomicUsize) {
    (AtomicBool::new(false), AtomicUsize::new(0))
}
#[test]
fn full_restore_keeps_current_backup_and_external_document_and_survives_missing_source() {
    let t = Temp::new();
    let (c, p) = state();
    let original = UserStorage::at(t.0.join("original")).unwrap();
    fs::write(original.root().join("materials/tip.bin"), b"tip").unwrap();
    fs::write(
        original.root().join("settings/preset.json"),
        serde_json::to_vec(&serde_json::json!({"path":original.root().join("materials/tip.bin")}))
            .unwrap(),
    )
    .unwrap();
    let doc = t.0.join("picture.drawer");
    fs::write(&doc, b"chosen document bytes").unwrap();
    let bundle = create(
        &original,
        &t.0.join("backup"),
        std::slice::from_ref(&doc),
        u64::MAX,
        &c,
        &p,
    )
    .unwrap();
    fs::remove_dir_all(original.root()).unwrap();
    let current = UserStorage::at(t.0.join("current")).unwrap();
    fs::write(current.root().join("settings/keep.txt"), b"current").unwrap();
    let out = restore(&current, &bundle, &t.0.join("restored"), u64::MAX, &c, &p).unwrap();
    assert_eq!(
        UserStorage::at(current.anchor().into()).unwrap().root(),
        out.root
    );
    assert_eq!(
        fs::read(out.root.join("restored-documents/picture.drawer")).unwrap(),
        b"chosen document bytes"
    );
    assert_eq!(fs::read(&doc).unwrap(), b"chosen document bytes");
    assert_eq!(
        fs::read(out.previous_backup.join("userdata/settings/keep.txt")).unwrap(),
        b"current"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(out.root.join("settings/preset.json")).unwrap()).unwrap();
    assert_eq!(
        value["path"],
        out.root
            .join("materials/tip.bin")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        fs::read(current.root().join("settings/keep.txt")).unwrap(),
        b"current"
    );
}
#[test]
fn corrupt_duplicate_traversal_and_unknown_version_preserve_active_root() {
    let t = Temp::new();
    let (c, p) = state();
    let s = UserStorage::at(t.0.join("data")).unwrap();
    s.save_brush(crate::assets::BrushTip::Pencil).unwrap();
    let b = create(&s, &t.0.join("backup"), &[], u64::MAX, &c, &p).unwrap();
    let manifest_path = b.join("bundle.json");
    let original = fs::read(&manifest_path).unwrap();
    for mode in 0..4 {
        let mut v: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match mode {
            0 => v["version"] = 99.into(),
            1 => {
                let first = v["files"][0].clone();
                v["files"].as_array_mut().unwrap().push(first);
            }
            2 => v["files"][0]["path"] = "../escape".into(),
            _ => v["files"][0]["path"] = "USERDATA".into(),
        }
        fs::write(&manifest_path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(
            restore(
                &s,
                &b,
                &t.0.join(format!("restore{mode}")),
                u64::MAX,
                &c,
                &p
            )
            .is_err()
        );
        assert_eq!(resolve(s.anchor()).unwrap(), s.root());
    }
    fs::write(&manifest_path, &original).unwrap();
    fs::write(b.join("userdata/settings/preferences.txt"), b"corrupt").unwrap();
    assert!(restore(&s, &b, &t.0.join("bad"), u64::MAX, &c, &p).is_err());
    assert_eq!(s.load_brush().unwrap(), crate::assets::BrushTip::Pencil);
}
#[test]
fn disk_cancel_duplicate_names_and_destination_collision_do_not_publish() {
    let t = Temp::new();
    let (c, p) = state();
    let s = UserStorage::at(t.0.join("data")).unwrap();
    assert!(create(&s, &t.0.join("low"), &[], 0, &c, &p).is_err());
    assert!(!t.0.join("low").exists());
    let b = create(&s, &t.0.join("backup"), &[], u64::MAX, &c, &p).unwrap();
    assert!(restore(&s, &b, &t.0.join("restore"), 0, &c, &p).is_err());
    assert!(!t.0.join("restore-before-restore").exists());
    fs::create_dir(t.0.join("occupied")).unwrap();
    fs::write(t.0.join("occupied/keep"), b"keep").unwrap();
    assert!(restore(&s, &b, &t.0.join("occupied"), u64::MAX, &c, &p).is_err());
    assert_eq!(fs::read(t.0.join("occupied/keep")).unwrap(), b"keep");
    for d in ["a", "b"] {
        fs::create_dir(t.0.join(d)).unwrap();
        fs::write(t.0.join(d).join("same.drawer"), b"doc").unwrap();
    }
    assert!(
        create(
            &s,
            &t.0.join("dup"),
            &[t.0.join("a/same.drawer"), t.0.join("b/same.drawer")],
            u64::MAX,
            &c,
            &p
        )
        .is_err()
    );
    assert!(!t.0.join("dup").exists());
    c.store(true, Ordering::Relaxed);
    assert!(create(&s, &t.0.join("cancel"), &[], u64::MAX, &c, &p).is_err());
    assert!(!t.0.join("cancel").exists());
    assert_eq!(resolve(s.anchor()).unwrap(), s.root());
}

#[test]
fn failed_copy_cleanup_preserves_unknown_and_changed_contents() {
    let t = Temp::new();
    let stage = t.0.join("stage");
    fs::create_dir(&stage).unwrap();
    write(&stage.join(".drawer-move-owner.json"), &"owner").unwrap();
    fs::write(stage.join("foreign"), b"keep").unwrap();
    discard_owned(&stage, "owner", &[], false);
    assert_eq!(fs::read(stage.join("foreign")).unwrap(), b"keep");
    fs::remove_file(stage.join("foreign")).unwrap();
    discard_owned(&stage, "owner", &[], false);
    assert!(!stage.exists());
}
