use serde_json::Value;

#[test]
fn fixture_scene_has_terrain_static_water_and_marker_batches() {
    let scene = world_renderer_native::fixtures::tiny_scene();
    let report = world_renderer_native::query::query_visible(
        &scene,
        world_renderer_native::model::CameraQuery::default(),
        world_renderer_native::model::RenderSettings::default(),
    );
    let value: Value = serde_json::to_value(report).unwrap();
    let batches = value["data"]["batches"].as_array().unwrap();

    assert!(batches.iter().any(|batch| batch["kind"] == "terrain"));
    assert!(batches.iter().any(|batch| batch["kind"] == "static"));
    assert!(batches.iter().any(|batch| batch["kind"] == "water"));
    assert!(batches.iter().any(|batch| batch["kind"] == "marker"));
}

#[test]
fn disabled_refs_are_filtered_by_default() {
    let scene = world_renderer_native::fixtures::tiny_scene();
    let report = world_renderer_native::query::query_visible(
        &scene,
        world_renderer_native::model::CameraQuery::default(),
        world_renderer_native::model::RenderSettings::default(),
    );
    let value: Value = serde_json::to_value(report).unwrap();

    assert_eq!(value["counts"]["disabled_refs"], 1);
    assert_eq!(value["counts"]["visible_instances"], 4);
}

#[test]
fn fixture_batch_manifest_only_advertises_existing_buffers() {
    let scene = world_renderer_native::fixtures::tiny_scene();
    let report = world_renderer_native::query::query_visible(
        &scene,
        world_renderer_native::model::CameraQuery::default(),
        world_renderer_native::model::RenderSettings::default(),
    );
    let value: Value = serde_json::to_value(report).unwrap();
    let batches = value["data"]["batches"].as_array().unwrap();

    for batch in batches {
        let mesh_buffer = batch["mesh_buffer"].as_str().unwrap();
        let instance_buffer = batch["instance_buffer"].as_str().unwrap();
        assert!(scene.buffers.contains_key(mesh_buffer), "{mesh_buffer}");
        assert!(
            scene.buffers.contains_key(instance_buffer),
            "{instance_buffer}"
        );
    }
}
