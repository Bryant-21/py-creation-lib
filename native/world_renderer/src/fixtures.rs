use crate::model::{
    BatchKind, CellBounds, MarkerPacket, MaterialDescriptor, TerrainTile, WaterSurface,
    WorldInstance, WorldScene,
};
use serde_json::json;
use std::collections::BTreeMap;

pub fn tiny_scene() -> WorldScene {
    let mut buffers = BTreeMap::new();
    buffers.insert("mesh:terrain:0".to_string(), vec![0, 1, 2, 3]);
    buffers.insert("instances:terrain:0".to_string(), vec![0, 0, 0, 0]);
    buffers.insert("mesh:static:0".to_string(), vec![1, 2, 3, 4]);
    buffers.insert("instances:static:0".to_string(), vec![1, 0, 0, 0]);
    buffers.insert("mesh:water:0".to_string(), vec![2, 3, 4, 5]);
    buffers.insert("instances:water:0".to_string(), vec![2, 0, 0, 0]);
    buffers.insert("mesh:marker:0".to_string(), vec![3, 4, 5, 6]);
    buffers.insert("instances:marker:0".to_string(), vec![3, 0, 0, 0]);
    buffers.insert("debug:form_id".to_string(), vec![1, 2, 3, 4]);
    buffers.insert("debug:depth".to_string(), vec![0, 0, 0, 255]);
    buffers.insert("debug:normal".to_string(), vec![127, 127, 255, 255]);
    buffers.insert("debug:material".to_string(), vec![16, 32, 48, 255]);
    buffers.insert("debug:diffuse".to_string(), vec![128, 128, 128, 255]);
    buffers.insert("debug:light".to_string(), vec![255, 255, 255, 255]);

    WorldScene {
        worldspace: "TinyWorld".to_string(),
        bounds: CellBounds::default(),
        instances: vec![
            WorldInstance {
                instance_id: 1,
                kind: BatchKind::Terrain,
                disabled: false,
                form_key: "Tiny.esm:000800".to_string(),
                base_form_key: "Tiny.esm:000800".to_string(),
                signature: "LAND".to_string(),
                source_plugin: "Tiny.esm".to_string(),
                cell: [0, 0],
                model_path: String::new(),
                position: [0.0, 0.0, 0.0],
                rotation_degrees: [0.0, 0.0, 0.0],
                scale: 1.0,
                layer_form_key: None,
                static_collection_parent: None,
            },
            WorldInstance {
                instance_id: 2,
                kind: BatchKind::Static,
                disabled: false,
                form_key: "Tiny.esm:000801".to_string(),
                base_form_key: "Tiny.esm:000100".to_string(),
                signature: "STAT".to_string(),
                source_plugin: "Tiny.esm".to_string(),
                cell: [0, 0],
                model_path: "meshes/tiny/crate.nif".to_string(),
                position: [128.0, 0.0, 0.0],
                rotation_degrees: [0.0, 0.0, 45.0],
                scale: 1.0,
                layer_form_key: None,
                static_collection_parent: None,
            },
            WorldInstance {
                instance_id: 3,
                kind: BatchKind::Water,
                disabled: false,
                form_key: "Tiny.esm:000802".to_string(),
                base_form_key: "Tiny.esm:000200".to_string(),
                signature: "WATR".to_string(),
                source_plugin: "Tiny.esm".to_string(),
                cell: [0, 0],
                model_path: String::new(),
                position: [0.0, 0.0, -16.0],
                rotation_degrees: [0.0, 0.0, 0.0],
                scale: 1.0,
                layer_form_key: None,
                static_collection_parent: None,
            },
            WorldInstance {
                instance_id: 4,
                kind: BatchKind::Marker,
                disabled: false,
                form_key: "Tiny.esm:000803".to_string(),
                base_form_key: "Tiny.esm:000300".to_string(),
                signature: "REFR".to_string(),
                source_plugin: "Tiny.esm".to_string(),
                cell: [0, 0],
                model_path: "marker:xmarker".to_string(),
                position: [0.0, 128.0, 0.0],
                rotation_degrees: [0.0, 0.0, 0.0],
                scale: 1.0,
                layer_form_key: None,
                static_collection_parent: None,
            },
            WorldInstance {
                instance_id: 5,
                kind: BatchKind::Static,
                disabled: true,
                form_key: "Tiny.esm:000804".to_string(),
                base_form_key: "Tiny.esm:000101".to_string(),
                signature: "STAT".to_string(),
                source_plugin: "Tiny.esm".to_string(),
                cell: [0, 0],
                model_path: "meshes/tiny/disabled.nif".to_string(),
                position: [-128.0, 0.0, 0.0],
                rotation_degrees: [0.0, 0.0, 0.0],
                scale: 1.0,
                layer_form_key: Some("Tiny.esm:000900".to_string()),
                static_collection_parent: None,
            },
        ],
        buffers,
        mesh_packets: Vec::new(),
        materials: vec![MaterialDescriptor {
            material_id: "static:default".to_string(),
            shader_model: "spec-gloss".to_string(),
            alpha_mode: "opaque".to_string(),
            diffuse_texture: None,
            normal_texture: None,
            specular_texture: None,
            env_texture: None,
        }],
        terrain_tiles: vec![TerrainTile {
            tile_id: "terrain:0:0".to_string(),
            cell: [0, 0],
            height_buffer: "mesh:terrain:0".to_string(),
            blend_buffer: "instances:terrain:0".to_string(),
            material_layers: vec!["default_land".to_string()],
        }],
        water_surfaces: vec![WaterSurface {
            water_id: "water:0:0".to_string(),
            cell: [0, 0],
            height: -16.0,
            material_id: "water:default".to_string(),
            color_rgba: [0.15, 0.25, 0.33, 0.65],
        }],
        markers: vec![MarkerPacket {
            marker_id: "marker:4".to_string(),
            instance_id: 4,
            marker_type: "xmarker".to_string(),
            position: [0.0, 128.0, 0.0],
        }],
        load_report: crate::model::Report::ok(json!({ "worldspace": "TinyWorld" })),
    }
}
