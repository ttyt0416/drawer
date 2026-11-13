use std::path::Path;
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    drawer_core::history_audit::run()?;
    drawer_core::history_navigation::verify::run()?;
    type Verify = fn(&Path) -> Result<(), String>;
    let groups: [(&str, Verify); 13] = [
        ("layers", drawer_core::layers_verify::verify),
        ("folders", drawer_core::layer_folders_verify::verify),
        ("groups", drawer_core::layer_groups_verify::verify),
        ("properties", drawer_core::layer_properties_verify::verify),
        ("selection", drawer_core::layer_selection_verify::verify),
        ("merge", drawer_core::layer_merge_verify::verify),
        ("masks", drawer_core::layer_masks_verify::verify),
        (
            "clipping-solo-rotation",
            drawer_core::clipping_verify::verify,
        ),
        ("material", drawer_core::material_layer_verify::verify),
        ("text-format", drawer_core::text_format_verify::verify),
        ("timing", drawer_core::timeline_timing_verify::verify),
        ("world-physics", drawer_core::world_physics_verify::verify),
        ("movement", drawer_core::layer_move_verify::verify),
    ];
    let mut checked = vec![];
    for (name, verify) in groups {
        let dir = root.join(name);
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        verify(&dir)?;
        checked.push(name);
    }
    Ok(
        serde_json::json!({"retainedTransactionFormats":["Pixel","Pixels","Multi","Scene","State"],"detachedAudit":true,"allRetainedUndoAndRedoPayloads":true,"featureRegressionGroups":checked,"newEditDiscardsRedoBranch":true,"savedDirtyBoundaryAndDocumentIsolation":true,"hiddenChildrenSurviveSolo":true,"deletedHiddenClipBaseDoesNotRetarget":true,"quarterTurnTextureAndLineClipping":true}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn retained_history_formats_feature_semantics_solo_hidden_clip_parent_and_rotation() {
        let r = crate::tests::TestDirectory::new();
        super::run(&r.0).unwrap();
    }
}
