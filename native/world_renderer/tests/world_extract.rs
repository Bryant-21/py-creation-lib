use serde_json::Value;

#[test]
fn empty_plugin_stack_lists_no_worldspaces_with_ok_report() {
    let session = world_renderer_native::model::WorldSession::synthetic("fo4");
    let report = world_renderer_native::records::list_worldspaces(&session).unwrap();
    let value: Value = serde_json::to_value(report).unwrap();

    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["worldspaces"].as_array().unwrap().len(), 0);
}

#[test]
fn invalid_plugin_path_is_reported_as_hard_error() {
    let session = world_renderer_native::model::WorldSession {
        game: world_renderer_native::model::SupportedGame::Fo4,
        plugin_paths: vec!["Z:/missing/Missing.esm".to_string()],
        data_paths: Vec::new(),
        archive_paths: Vec::new(),
    };
    let err = world_renderer_native::records::list_worldspaces(&session).unwrap_err();

    assert!(err.to_string().contains("Missing.esm"));
}

#[test]
fn renderable_signatures_are_classified() {
    assert!(world_renderer_native::records::is_static_renderable_signature("STAT"));
    assert!(world_renderer_native::records::is_static_renderable_signature("SCOL"));
    assert!(world_renderer_native::records::is_static_renderable_signature("MSTT"));
    assert!(world_renderer_native::records::is_static_renderable_signature("TREE"));
    assert!(world_renderer_native::records::is_static_renderable_signature("FLOR"));
    assert!(world_renderer_native::records::is_static_renderable_signature("ACTI"));
    assert!(world_renderer_native::records::is_static_renderable_signature("DOOR"));
    assert!(world_renderer_native::records::is_static_renderable_signature("FURN"));
    assert!(world_renderer_native::records::is_static_renderable_signature("LIGH"));
    assert!(!world_renderer_native::records::is_static_renderable_signature("NPC_"));
}

#[test]
fn asset_cache_key_normalizes_mesh_paths() {
    let key_a = world_renderer_native::assets::AssetCacheKey::new("Meshes\\Foo\\Bar.NIF", "loose:");
    let key_b = world_renderer_native::assets::AssetCacheKey::new("meshes/foo/bar.nif", "loose:");

    assert_eq!(key_a, key_b);
}

#[test]
fn missing_model_generates_warning_packet() {
    let session = world_renderer_native::model::WorldSession::synthetic("fo4");
    let mut scene = world_renderer_native::fixtures::tiny_scene();
    scene.instances[1].model_path = "meshes/missing/not_found.nif".to_string();

    let report =
        world_renderer_native::geometry::prepare_scene_geometry(&session, &mut scene).unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.code == "missing_model")
    );
}

#[test]
fn terrain_water_and_markers_are_counted_in_scene_stats() {
    let scene = world_renderer_native::fixtures::tiny_scene();
    let stats = world_renderer_native::query::scene_stats(&scene);
    let value = serde_json::to_value(stats).unwrap();

    assert_eq!(value["counts"]["terrain_tiles"], 1);
    assert_eq!(value["counts"]["water_surfaces"], 1);
    assert_eq!(value["counts"]["markers"], 1);
}
