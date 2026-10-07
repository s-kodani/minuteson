// Include in bootstrap/placement/activation/tests/native_history.rs of the
// f930f4f0c2dd1c2502c8515f9e1d96624e7050c7 source export, not the current app.
// This adds only a synthetic fixture producer to the historical test module.
#[test]
#[ignore = "requires MON_HISTORY_V27_FIXTURE_OUTPUT; synthetic cross-version fixture export"]
fn history_export_v27_selected_move_fixtures() {
    let output = std::env::var("MON_HISTORY_V27_FIXTURE_OUTPUT").unwrap();
    assert!(!Path::new(&output).exists());
    let mut records = Vec::new();
    for full in [false, true] {
        let mut f = NativeFixture::new(false, full);
        let (model_operation, model_file) = synthetic_model(&f, true);
        std::fs::write(
            f._base.path().join(".mon-v27-cross-version-fixture"),
            b"f930f4f0c2dd1c2502c8515f9e1d96624e7050c7",
        )
        .unwrap();
        f.select();
        let record = serde_json::json!({
            "schemaVersion":27,"sourceRevision":"f930f4f0c2dd1c2502c8515f9e1d96624e7050c7",
            "root":f._base.path(),"config":f.config,"legacy":f.legacy,"target":f.target,
            "meeting":f.meeting,"deletion":f.deletion,"fullDeletion":f.full_deletion,
            "modelOperation":model_operation,"modelFile":model_file
        });
        // Deliberately retain only these synthetic fixtures for the separate
        // current-version consumer; its operator removes them after testing.
        let retained = f._base.keep();
        assert!(retained.exists());
        records.push(record);
    }
    std::fs::write(output, serde_json::to_vec(&records).unwrap()).unwrap();
}
