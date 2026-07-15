use crate::model::{MarkerPacket, TerrainTile, WaterSurface, WorldScene, WorldSession};

pub fn extract_land_tiles(_session: &WorldSession, _worldspace: &str) -> Vec<TerrainTile> {
    Vec::new()
}

pub fn extract_fo76_btd_tiles(_session: &WorldSession, _worldspace: &str) -> Vec<TerrainTile> {
    Vec::new()
}

pub fn extract_water_surfaces(_session: &WorldSession, _worldspace: &str) -> Vec<WaterSurface> {
    Vec::new()
}

pub fn marker_packets_from_instances(scene: &WorldScene) -> Vec<MarkerPacket> {
    scene
        .instances
        .iter()
        .filter(|instance| instance.kind == crate::model::BatchKind::Marker)
        .map(|instance| MarkerPacket {
            marker_id: format!("marker:{}", instance.instance_id),
            instance_id: instance.instance_id,
            marker_type: instance
                .model_path
                .trim_start_matches("marker:")
                .to_string(),
            position: instance.position,
        })
        .collect()
}
