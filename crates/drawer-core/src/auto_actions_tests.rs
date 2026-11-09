#[test]
fn sets_actions_steps_global_ids_empty_last_delete_and_self_copy() {
    super::super::auto_actions_verify::editing().unwrap();
}
#[test]
fn parameterized_replay_atomic_failure_cancel_stale_and_one_undo() {
    super::super::auto_actions_verify::playback().unwrap();
}
#[test]
fn damaged_files_versions_duplicate_ids_foreign_tools_and_local_exchange() {
    let t = crate::test_support::TempDir::new();
    super::super::auto_actions_verify::files(&t.0).unwrap();
}
#[test]
fn all_recordable_operations_compose_in_order_and_unicode_label_is_bounded() {
    super::super::auto_actions_verify::variants().unwrap();
}
