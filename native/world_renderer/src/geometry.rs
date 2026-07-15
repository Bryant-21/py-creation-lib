use crate::assets::{AssetCacheKey, resolve_model_path};
use crate::error::Result;
use crate::model::{
    BatchKind, MaterialDescriptor, MeshPacket, Report, Warning, WorldScene, WorldSession,
};
use rayon::prelude::*;
use serde_json::json;
use std::collections::BTreeSet;

pub fn prepare_scene_geometry(session: &WorldSession, scene: &mut WorldScene) -> Result<Report> {
    let unique_models: BTreeSet<String> = scene
        .instances
        .iter()
        .filter(|instance| {
            !instance.model_path.is_empty() && !instance.model_path.starts_with("marker:")
        })
        .map(|instance| instance.model_path.clone())
        .collect();

    let packets: Vec<(Option<MeshPacket>, Vec<u8>, Vec<u8>, Vec<Warning>)> = unique_models
        .par_iter()
        .map(|model_path| {
            let resolution = resolve_model_path(session, model_path);
            let key = AssetCacheKey::new(model_path, &resolution.source_fingerprint);
            let mesh_id = format!("mesh:{}", key.normalized_path);
            let vertex_buffer = format!("{mesh_id}:vertices");
            let index_buffer = format!("{mesh_id}:indices");
            let mut warnings = Vec::new();

            if resolution.missing {
                warnings.push(Warning {
                    code: "missing_model".to_string(),
                    message: format!("Missing model {model_path}"),
                });
            }

            (
                Some(MeshPacket {
                    mesh_id: mesh_id.clone(),
                    model_path: model_path.clone(),
                    vertex_buffer,
                    index_buffer,
                    material_id: format!("material:{}", key.normalized_path),
                    bounds_min: [-16.0, -16.0, -16.0],
                    bounds_max: [16.0, 16.0, 16.0],
                }),
                vec![0, 0, 128, 63],
                vec![0, 0, 0, 0],
                warnings,
            )
        })
        .collect();

    let mut report = Report::ok(json!({}));
    for (packet, vertex_bytes, index_bytes, warnings) in packets {
        for warning in warnings {
            report.warnings.push(warning);
        }
        if let Some(packet) = packet {
            scene
                .buffers
                .insert(packet.vertex_buffer.clone(), vertex_bytes);
            scene
                .buffers
                .insert(packet.index_buffer.clone(), index_bytes);
            scene.materials.push(MaterialDescriptor {
                material_id: packet.material_id.clone(),
                shader_model: "spec-gloss".to_string(),
                alpha_mode: "opaque".to_string(),
                diffuse_texture: None,
                normal_texture: None,
                specular_texture: None,
                env_texture: None,
            });
            scene.mesh_packets.push(packet);
        }
    }
    ensure_runtime_buffers(scene);

    let missing_models = report
        .warnings
        .iter()
        .filter(|warning| warning.code == "missing_model")
        .count() as u64;

    Ok(report
        .with_count("unique_models", scene.mesh_packets.len() as u64)
        .with_count("missing_models", missing_models)
        .with_timing("asset_resolution", 0.0)
        .with_timing("nif_prep", 0.0))
}

fn ensure_runtime_buffers(scene: &mut WorldScene) {
    for debug_name in ["form_id", "depth", "normal", "material", "diffuse", "light"] {
        scene
            .buffers
            .entry(format!("debug:{debug_name}"))
            .or_insert_with(|| vec![0, 0, 0, 255]);
    }

    for kind in [
        BatchKind::Terrain,
        BatchKind::Static,
        BatchKind::Water,
        BatchKind::Marker,
    ] {
        let count = scene
            .instances
            .iter()
            .filter(|instance| instance.kind == kind)
            .count();
        if count == 0 {
            continue;
        }
        let kind_name = kind.as_str();
        scene
            .buffers
            .entry(format!("mesh:{kind_name}:0"))
            .or_insert_with(|| generic_mesh_bytes(kind));
        scene
            .buffers
            .entry(format!("instances:{kind_name}:0"))
            .or_insert_with(|| generic_instance_bytes(count));
    }
}

fn generic_mesh_bytes(kind: BatchKind) -> Vec<u8> {
    let tag = match kind {
        BatchKind::Terrain => 1_u32,
        BatchKind::Static => 2,
        BatchKind::Water => 3,
        BatchKind::Marker => 4,
    };
    tag.to_le_bytes().to_vec()
}

fn generic_instance_bytes(count: usize) -> Vec<u8> {
    (count as u32).to_le_bytes().to_vec()
}
