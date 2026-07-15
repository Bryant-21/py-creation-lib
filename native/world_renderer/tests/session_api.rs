use serde_json::Value;

#[test]
fn report_envelope_serializes_counts_and_data() {
    let report = world_renderer_native::model::Report::ok(serde_json::json!({
        "worldspaces": []
    }))
    .with_count("worldspaces", 0);

    let value: Value = serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["errors"].as_array().unwrap().len(), 0);
    assert_eq!(value["counts"]["worldspaces"], 0);
    assert!(value["data"]["worldspaces"].as_array().unwrap().is_empty());
}

#[test]
fn handle_registry_allocates_and_removes_sessions() {
    let registry = world_renderer_native::handles::HandleRegistry::default();
    let id = registry.insert_session(world_renderer_native::model::WorldSession::synthetic("fo4"));

    assert!(registry.with_session(id, |_| ()).is_ok());
    assert!(registry.remove_session(id).is_ok());
    assert!(registry.with_session(id, |_| ()).is_err());
}
