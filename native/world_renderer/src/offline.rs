use crate::error::{Result, WorldRendererError};
use crate::model::{OfflineRenderJob, Report, WorldScene};
use serde_json::json;

pub fn render_offline(scene: &WorldScene, job: OfflineRenderJob) -> Result<Report> {
    if job.width == 0 || job.height == 0 {
        return Err(WorldRendererError::Message(
            "width and height must be positive".to_string(),
        ));
    }

    Ok(Report::ok(json!({
        "output_path": job.output_path,
        "width": job.width,
        "height": job.height,
        "worldspace": scene.worldspace,
    }))
    .with_count("instances", scene.instances.len() as u64)
    .with_count("terrain_tiles", scene.terrain_tiles.len() as u64)
    .with_count("water_surfaces", scene.water_surfaces.len() as u64)
    .with_count("markers", scene.markers.len() as u64)
    .with_timing("culling", 0.0)
    .with_timing("draw", 0.0))
}
