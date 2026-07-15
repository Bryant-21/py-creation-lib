use crate::model::{BatchKind, CameraQuery, RenderSettings, Report, WorldBatch, WorldScene};
use serde_json::json;
use std::collections::BTreeMap;

pub fn query_visible(scene: &WorldScene, _camera: CameraQuery, settings: RenderSettings) -> Report {
    let mut by_kind: BTreeMap<BatchKind, u32> = BTreeMap::new();
    let mut disabled_refs = 0_u64;

    for instance in &scene.instances {
        if instance.disabled && !settings.include_disabled_refs {
            disabled_refs += 1;
            continue;
        }
        match instance.kind {
            BatchKind::Terrain if settings.include_terrain => {
                *by_kind.entry(BatchKind::Terrain).or_default() += 1
            }
            BatchKind::Static if settings.include_statics => {
                *by_kind.entry(BatchKind::Static).or_default() += 1
            }
            BatchKind::Water if settings.include_water => {
                *by_kind.entry(BatchKind::Water).or_default() += 1
            }
            BatchKind::Marker if settings.include_markers => {
                *by_kind.entry(BatchKind::Marker).or_default() += 1
            }
            _ => {}
        }
    }

    let batches: Vec<WorldBatch> = by_kind
        .iter()
        .map(|(kind, count)| {
            let kind_name = kind.as_str();
            let mut debug_buffers = BTreeMap::new();
            for debug_name in ["form_id", "depth", "normal", "material", "diffuse", "light"] {
                debug_buffers.insert(debug_name.to_string(), format!("debug:{debug_name}"));
            }
            WorldBatch {
                id: format!("{kind_name}:0"),
                kind: *kind,
                mesh_buffer: format!("mesh:{kind_name}:0"),
                instance_buffer: format!("instances:{kind_name}:0"),
                material_id: format!("{kind_name}:default"),
                instance_count: *count,
                pick_buffer: Some("debug:form_id".to_string()),
                debug_buffers,
            }
        })
        .collect();

    Report::ok(json!({ "batches": batches }))
        .with_count(
            "visible_instances",
            by_kind.values().map(|v| u64::from(*v)).sum(),
        )
        .with_count("disabled_refs", disabled_refs)
        .with_timing("culling", 0.0)
}

pub fn scene_stats(scene: &WorldScene) -> Report {
    let disabled_refs = scene
        .instances
        .iter()
        .filter(|instance| instance.disabled)
        .count() as u64;

    let mut report = Report::ok(json!({
        "worldspace": scene.worldspace,
        "bounds": scene.bounds,
        "warnings": scene.load_report.warnings,
    }))
    .with_count("cells", cell_count(scene) as u64)
    .with_count("instances", scene.instances.len() as u64)
    .with_count("placed_refs", scene.instances.len() as u64)
    .with_count("disabled_refs", disabled_refs)
    .with_count("terrain_tiles", scene.terrain_tiles.len() as u64)
    .with_count("water_surfaces", scene.water_surfaces.len() as u64)
    .with_count("markers", scene.markers.len() as u64)
    .with_count("unique_models", scene.mesh_packets.len() as u64)
    .with_count("buffers", scene.buffers.len() as u64);
    report.warnings = scene.load_report.warnings.clone();
    report.timings_ms = scene.load_report.timings_ms.clone();
    report
}

fn cell_count(scene: &WorldScene) -> usize {
    let width = (scene.bounds.max_x - scene.bounds.min_x + 1).max(0) as usize;
    let height = (scene.bounds.max_y - scene.bounds.min_y + 1).max(0) as usize;
    width * height
}
