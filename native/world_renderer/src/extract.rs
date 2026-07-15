use crate::error::Result;
use crate::geometry;
use crate::model::{
    BatchKind, CellBounds, MaterialDescriptor, RenderSettings, Report, TerrainTile, WaterSurface,
    WorldInstance, WorldScene, WorldSession,
};
use crate::records;
use esp_authoring_core::plugin_runtime::{ParsedGroup, ParsedPlugin, ParsedRecord};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

pub fn load_worldspace(
    session: &WorldSession,
    worldspace: &str,
    bounds: CellBounds,
    settings: RenderSettings,
) -> Result<WorldScene> {
    let mut scene = if session.plugin_paths.is_empty() {
        crate::fixtures::tiny_scene()
    } else {
        load_worldspace_from_plugins(session, worldspace, bounds)?
    };
    scene.worldspace = worldspace.to_string();
    scene.bounds = bounds;
    if !settings.include_markers {
        scene.markers.clear();
    }
    let geometry_report = geometry::prepare_scene_geometry(session, &mut scene)?;
    scene.merge_report(geometry_report);
    Ok(scene)
}

fn load_worldspace_from_plugins(
    session: &WorldSession,
    worldspace: &str,
    bounds: CellBounds,
) -> Result<WorldScene> {
    let plugins = records::parse_plugins(session)?;
    Ok(load_worldspace_from_parsed_plugins(
        &plugins, worldspace, bounds,
    ))
}

fn load_worldspace_from_parsed_plugins(
    plugins: &[ParsedPlugin],
    worldspace: &str,
    bounds: CellBounds,
) -> WorldScene {
    let base_models = collect_base_models(plugins);
    let mut scene = WorldScene::empty(worldspace, bounds);
    let mut next_instance_id = 1_u64;
    let mut selected_cells = 0_u64;
    let mut found_world = false;
    let mut report = Report::ok(json!({ "worldspace": worldspace }));

    for plugin in plugins {
        let Some(wrld_group) = records::top_group(plugin, "WRLD") else {
            continue;
        };
        let Some(world_record) = records::find_world(plugin, worldspace) else {
            continue;
        };
        found_world = true;
        let Some(world_children) =
            records::find_world_children_group(wrld_group, world_record.form_id)
        else {
            report.push_warning(
                "missing_world_children",
                format!(
                    "{} has WRLD {} but no child group",
                    plugin.plugin_name,
                    records::render_form_key(plugin, world_record.form_id)
                ),
            );
            continue;
        };
        let mut child_groups = BTreeMap::new();
        records::collect_cell_child_groups(world_children, &mut child_groups);
        let mut cell_records = Vec::new();
        records::collect_group_records(world_children, "CELL", &mut cell_records);

        for cell_record in cell_records {
            let Some(cell) = records::cell_grid(cell_record) else {
                continue;
            };
            if !records::inside_bounds(cell, bounds) {
                continue;
            }
            selected_cells += 1;
            let child_group = child_groups.get(&cell_record.form_id).copied();
            collect_cell_terrain(plugin, child_group, cell, &mut scene, &mut next_instance_id);
            collect_cell_water(plugin, cell_record, cell, &mut scene, &mut next_instance_id);
            collect_cell_instances(
                plugin,
                child_group,
                cell,
                &base_models,
                &mut scene.instances,
                &mut next_instance_id,
            );
        }
    }

    scene.markers = crate::terrain::marker_packets_from_instances(&scene);
    if scene
        .terrain_tiles
        .iter()
        .any(|tile| !tile.material_layers.is_empty())
    {
        scene.materials.push(MaterialDescriptor {
            material_id: "terrain:default".to_string(),
            shader_model: "terrain".to_string(),
            alpha_mode: "opaque".to_string(),
            diffuse_texture: None,
            normal_texture: None,
            specular_texture: None,
            env_texture: None,
        });
    }
    if !scene.water_surfaces.is_empty() {
        scene.materials.push(MaterialDescriptor {
            material_id: "water:default".to_string(),
            shader_model: "water".to_string(),
            alpha_mode: "blend".to_string(),
            diffuse_texture: None,
            normal_texture: None,
            specular_texture: None,
            env_texture: None,
        });
    }
    if !found_world {
        report.push_warning(
            "worldspace_not_found",
            format!("worldspace not found in plugin stack: {worldspace}"),
        );
    }
    scene.load_report = report
        .with_count("plugins", plugins.len() as u64)
        .with_count("placed_refs", scene.instances.len() as u64)
        .with_count("cells_loaded", selected_cells)
        .with_count("terrain_tiles", scene.terrain_tiles.len() as u64)
        .with_count("water_surfaces", scene.water_surfaces.len() as u64)
        .with_count("markers", scene.markers.len() as u64)
        .with_timing("plugin_load", 0.0)
        .with_timing("cell_extraction", 0.0);
    scene
}

fn collect_base_models(plugins: &[ParsedPlugin]) -> HashMap<String, String> {
    let mut base_models = HashMap::new();
    for plugin in plugins {
        let mut all = Vec::new();
        records::all_records(&plugin.root_items, &mut all);
        for record in all {
            if records::is_static_renderable_signature(record.signature.as_str()) {
                if let Some(model_path) = records::subrecord_zstring(record, "MODL") {
                    base_models
                        .insert(records::render_form_key(plugin, record.form_id), model_path);
                }
            }
        }
    }
    base_models
}

fn collect_cell_instances(
    plugin: &ParsedPlugin,
    child_group: Option<&ParsedGroup>,
    cell: [i32; 2],
    base_models: &HashMap<String, String>,
    instances: &mut Vec<WorldInstance>,
    next_instance_id: &mut u64,
) {
    let Some(child_group) = child_group else {
        return;
    };
    let mut placed_records = Vec::new();
    records::collect_placed_records(child_group, &mut placed_records);
    for record in placed_records {
        if let Some(instance) =
            instance_from_record(plugin, record, base_models, *next_instance_id, cell)
        {
            instances.push(instance);
            *next_instance_id += 1;
        }
    }
}

fn collect_cell_terrain(
    plugin: &ParsedPlugin,
    child_group: Option<&ParsedGroup>,
    cell: [i32; 2],
    scene: &mut WorldScene,
    next_instance_id: &mut u64,
) {
    let Some(child_group) = child_group else {
        return;
    };
    let mut land_records = Vec::new();
    records::collect_group_records(child_group, "LAND", &mut land_records);
    for land_record in land_records {
        let tile_id = format!("terrain:{}:{}", cell[0], cell[1]);
        scene.terrain_tiles.push(TerrainTile {
            tile_id: tile_id.clone(),
            cell,
            height_buffer: "mesh:terrain:0".to_string(),
            blend_buffer: "instances:terrain:0".to_string(),
            material_layers: vec!["terrain:default".to_string()],
        });
        scene.instances.push(WorldInstance {
            instance_id: *next_instance_id,
            kind: BatchKind::Terrain,
            disabled: false,
            form_key: records::render_form_key(plugin, land_record.form_id),
            base_form_key: records::render_form_key(plugin, land_record.form_id),
            signature: "LAND".to_string(),
            source_plugin: plugin.plugin_name.clone(),
            cell,
            model_path: String::new(),
            position: [cell[0] as f32 * 4096.0, cell[1] as f32 * 4096.0, 0.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            scale: 1.0,
            layer_form_key: None,
            static_collection_parent: None,
        });
        *next_instance_id += 1;
    }
}

fn collect_cell_water(
    plugin: &ParsedPlugin,
    cell_record: &ParsedRecord,
    cell: [i32; 2],
    scene: &mut WorldScene,
    next_instance_id: &mut u64,
) {
    let Some(height) = records::subrecord_f32(cell_record, "XCLW") else {
        return;
    };
    let water_id = format!("water:{}:{}", cell[0], cell[1]);
    scene.water_surfaces.push(WaterSurface {
        water_id: water_id.clone(),
        cell,
        height,
        material_id: "water:default".to_string(),
        color_rgba: [0.15, 0.25, 0.33, 0.65],
    });
    scene.instances.push(WorldInstance {
        instance_id: *next_instance_id,
        kind: BatchKind::Water,
        disabled: false,
        form_key: records::render_form_key(plugin, cell_record.form_id),
        base_form_key: records::render_form_key(plugin, cell_record.form_id),
        signature: "WATR".to_string(),
        source_plugin: plugin.plugin_name.clone(),
        cell,
        model_path: String::new(),
        position: [cell[0] as f32 * 4096.0, cell[1] as f32 * 4096.0, height],
        rotation_degrees: [0.0, 0.0, 0.0],
        scale: 1.0,
        layer_form_key: None,
        static_collection_parent: None,
    });
    *next_instance_id += 1;
}

fn instance_from_record(
    plugin: &ParsedPlugin,
    record: &ParsedRecord,
    base_models: &HashMap<String, String>,
    instance_id: u64,
    cell: [i32; 2],
) -> Option<WorldInstance> {
    let signature = record.signature.as_str();
    if !records::is_placed_child_signature(signature) {
        return None;
    }

    let (base_form_key, model_path) = match records::subrecord_u32(record, "NAME") {
        Some(base_raw) => {
            let base_form_key = records::render_form_key(plugin, base_raw);
            let model_path = base_models
                .get(&base_form_key)
                .cloned()
                .unwrap_or_else(|| format!("marker:{}", signature.to_ascii_lowercase()));
            (base_form_key, model_path)
        }
        None => (
            records::render_form_key(plugin, record.form_id),
            format!("marker:{}", signature.to_ascii_lowercase()),
        ),
    };
    let kind = if model_path.starts_with("marker:") {
        BatchKind::Marker
    } else {
        BatchKind::Static
    };
    let (position, rotation_degrees) =
        records::refr_data_transform(record).unwrap_or(([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
    let scale = records::subrecord_f32(record, "XSCL").unwrap_or(1.0);
    let layer_form_key =
        records::subrecord_u32(record, "XLYR").map(|raw| records::render_form_key(plugin, raw));

    Some(WorldInstance {
        instance_id,
        kind,
        disabled: (record.flags & 0x0000_0800) != 0,
        form_key: records::render_form_key(plugin, record.form_id),
        base_form_key,
        signature: signature.to_string(),
        source_plugin: plugin.plugin_name.clone(),
        cell,
        model_path,
        position,
        rotation_degrees,
        scale,
        layer_form_key,
        static_collection_parent: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use esp_authoring_core::plugin_runtime::{ParsedItem, ParsedPluginHeader, ParsedSubrecord};
    use smol_str::SmolStr;

    fn subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn zstring(value: &str) -> Vec<u8> {
        let mut data = value.as_bytes().to_vec();
        data.push(0);
        data
    }

    fn record(signature: &str, form_id: u32, editor_id: Option<&str>) -> ParsedRecord {
        let mut subrecords = Vec::new();
        if let Some(editor_id) = editor_id {
            subrecords.push(subrecord("EDID", zstring(editor_id)));
        }
        ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn cell_record(form_id: u32, editor_id: &str, x: i32, y: i32) -> ParsedRecord {
        let mut record = record("CELL", form_id, Some(editor_id));
        let mut grid = Vec::new();
        grid.extend_from_slice(&x.to_le_bytes());
        grid.extend_from_slice(&y.to_le_bytes());
        record.subrecords.push(subrecord("XCLC", grid));
        record
    }

    fn placed_ref(form_id: u32, base_form_id: u32, x: f32) -> ParsedRecord {
        let mut record = record("REFR", form_id, None);
        record
            .subrecords
            .push(subrecord("NAME", base_form_id.to_le_bytes().to_vec()));
        let mut data = Vec::new();
        for value in [x, 0.0, 0.0, 0.0, 0.0, 0.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        record.subrecords.push(subrecord("DATA", data));
        record
    }

    fn group(label: [u8; 4], group_type: i32, children: Vec<ParsedItem>) -> ParsedItem {
        ParsedItem::Group(ParsedGroup {
            label,
            group_type,
            tail: Bytes::new(),
            children,
        })
    }

    fn top_group(signature: &str, children: Vec<ParsedItem>) -> ParsedItem {
        let bytes = signature.as_bytes();
        group([bytes[0], bytes[1], bytes[2], bytes[3]], 0, children)
    }

    fn plugin(root_items: Vec<ParsedItem>) -> ParsedPlugin {
        ParsedPlugin {
            plugin_name: "Tiny.esm".to_string(),
            file_path: String::new(),
            header_size: 24,
            header: ParsedPluginHeader {
                version: 1.0,
                num_records: 0,
                next_object_id: 0x800,
                author: String::new(),
                description: String::new(),
                masters: Vec::new(),
                master_sizes: Vec::new(),
                overridden_forms: Vec::new(),
                flags: 0,
                extra_subrecords: Vec::new(),
                version_control: 0,
                form_version: None,
                version2: None,
                hedr_raw: None,
                raw_subrecords: Vec::new(),
            },
            root_items,
            game: Some("fo4".to_string()),
        }
    }

    #[test]
    fn parsed_plugin_extraction_is_scoped_to_requested_world_and_bounds() {
        let base = {
            let mut record = record("STAT", 0x00000800, Some("TinyStatic"));
            record
                .subrecords
                .push(subrecord("MODL", zstring("meshes/tiny/static.nif")));
            record
        };
        let world = record("WRLD", 0x00001000, Some("TinyWorld"));
        let in_cell = cell_record(0x00002000, "InBounds", 0, 0);
        let out_cell = cell_record(0x00002001, "OutBounds", 9, 9);
        let in_ref = placed_ref(0x00003000, 0x00000800, 128.0);
        let out_ref = placed_ref(0x00003001, 0x00000800, 256.0);
        let in_child_group = group(
            0x00002000_u32.to_le_bytes(),
            records::CELL_CHILD_GROUP,
            vec![
                ParsedItem::Record(record("LAND", 0x00004000, None)),
                group(
                    0x00002000_u32.to_le_bytes(),
                    records::TEMPORARY_GROUP,
                    vec![ParsedItem::Record(in_ref)],
                ),
            ],
        );
        let out_child_group = group(
            0x00002001_u32.to_le_bytes(),
            records::CELL_CHILD_GROUP,
            vec![group(
                0x00002001_u32.to_le_bytes(),
                records::TEMPORARY_GROUP,
                vec![ParsedItem::Record(out_ref)],
            )],
        );
        let world_children = group(
            0x00001000_u32.to_le_bytes(),
            records::WORLD_CHILD_GROUP,
            vec![
                ParsedItem::Record(in_cell),
                ParsedItem::Record(out_cell),
                in_child_group,
                out_child_group,
            ],
        );
        let plugin = plugin(vec![
            top_group("STAT", vec![ParsedItem::Record(base)]),
            top_group("WRLD", vec![ParsedItem::Record(world), world_children]),
        ]);

        let scene = load_worldspace_from_parsed_plugins(
            &[plugin],
            "TinyWorld",
            CellBounds {
                min_x: 0,
                min_y: 0,
                max_x: 0,
                max_y: 0,
            },
        );

        assert_eq!(scene.terrain_tiles.len(), 1);
        assert!(scene.instances.iter().any(|instance| {
            instance.form_key == "Tiny.esm:003000"
                && instance.cell == [0, 0]
                && instance.model_path == "meshes/tiny/static.nif"
        }));
        assert!(
            !scene
                .instances
                .iter()
                .any(|instance| instance.form_key == "Tiny.esm:003001")
        );
        assert_eq!(scene.load_report.counts["cells_loaded"], 1);
    }
}
