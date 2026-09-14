use std::collections::{HashMap, HashSet};
use std::path::Path;

use indexmap::IndexMap;
use regex::{NoExpand, Regex, RegexBuilder};
use serde_json::{Value, json};

use crate::model::{NifBlock, NifFile, NifValue};
use crate::schema::SCHEMA;
use crate::skin::pack::recompute_tangents_lengyel;

const VF_NORMALS: u64 = 0x0008;
const VF_TANGENTS: u64 = 0x0010;
const SLSF1_MODEL_SPACE_NORMALS: u64 = 1 << 12;
const LEGACY_HAS_TANGENTS: u64 = 1 << 12;
const OBLIVION_TANGENT_DATA_NAME: &str = "Tangent space (binormal & tangent vectors)";

pub fn process_nif_file(
    input: &Path,
    output: &Path,
    processor: &str,
    options_json: &str,
) -> Result<Value, String> {
    let options = if options_json.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(options_json).map_err(|error| error.to_string())?
    };
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if processor == "json-converter" {
        return crate::nif_json::convert_json_file(input, output, &options);
    }
    let material_file =
        extension.eq_ignore_ascii_case("bgsm") || extension.eq_ignore_ascii_case("bgem");
    if processor == "universal-tweaker" && material_file {
        return crate::universal_tweaker::tweak_material_file(input, output, &options);
    }
    if processor == "replace-assets" && material_file {
        return crate::universal_tweaker::replace_material_assets(input, output, &options);
    }
    let mut nif = NifFile::load(input).map_err(|error| error.to_string())?;
    let changes = match processor {
        "ps4-converter" => crate::ps4::convert_nif_to_ps4(&mut nif, input)?,
        "update-tangents" => update_tangents(&mut nif, &options),
        "optimize-mesh" => optimize_mesh(&mut nif, &options)?,
        "universal-fixer" => crate::validation::sanitize_nif(&mut nif).changes,
        "universal-tweaker" => crate::universal_tweaker::tweak_nif(&mut nif, &options)?,
        "update-bounds" => update_bounds(&mut nif),
        "replace-assets" => replace_assets(&mut nif, &options)?,
        "remove-unused-nodes" => remove_unused_nodes(&mut nif, &options),
        "convert-block-type" => convert_block_types(&mut nif, &options)?,
        "set-missing-names" => set_missing_names(&mut nif, input, &options),
        "unskin-mesh" => unskin_mesh(&mut nif),
        "update-shader-flags" => update_shader_flags(&mut nif, &options)?,
        "walls-reflection-flag" => walls_reflection_flag(&mut nif, &options),
        "soft-particles" => soft_particles(&mut nif, &options),
        "update-ragdoll-constraint" => update_ragdoll_constraints(&mut nif, &options)?,
        "update-havok-settings" => update_havok_settings(&mut nif, &options)?,
        "update-havok-inertia" => update_havok_inertia(&mut nif, &options)?,
        "search-havok-material" => search_havok_material(&mut nif, &options)?,
        "copy-controlled-blocks" => copy_controlled_blocks(&mut nif, &options)?,
        "copy-priorities" => copy_priorities(&mut nif, &options)?,
        "remove-controlled-blocks" => remove_controlled_blocks(&mut nif, &options)?,
        "quadratic-to-linear" => quadratic_to_linear(&mut nif, &options)?,
        "fix-exported-kf" => fix_exported_kf(&mut nif),
        "optimize-animations" => optimize_animations(&mut nif),
        "add-transform-data" => add_transform_data(&mut nif, &options)?,
        "add-headtracking-anim" => add_headtracking_anim(&mut nif, &options),
        "add-facial-anim" => add_facial_anim(&mut nif, &options)?,
        "weijiesen-blow-up" => weijiesen_blow_up(&mut nif),
        "add-skeleton-blocks" => add_skeleton_blocks(&mut nif, input, &options)?,
        "update-mopp-code" => update_mopp_code(&mut nif)?,
        "remove-nodes" => remove_nodes(&mut nif, &options)?,
        "attach-parent" => attach_parent(&mut nif, &options)?,
        "adjust-transform" => adjust_transform(&mut nif, &options)?,
        "copy-geometry-blocks" => copy_geometry_blocks(&mut nif, &options)?,
        "merge-properties" => merge_properties(&mut nif, &options)?,
        "group-shapes" => group_shapes(&mut nif, &options),
        "vertex-paint" => vertex_paint(&mut nif, &options)?,
        "merge-shapes" => merge_shapes(&mut nif, &options)?,
        "apply-transform" => apply_transforms(&mut nif, &options),
        "add-root-collision-node" => add_root_collision_node(&mut nif),
        "add-bounding-box" => add_bounding_box(&mut nif, &options),
        "add-lod-node" => add_lod_node(&mut nif, &options)?,
        _ => return Err(format!("Unsupported NIF processor: {processor}")),
    };
    let report_only = option_bool(&options, "report_only").unwrap_or(false)
        || (processor == "search-havok-material"
            && options
                .get("material_replace")
                .is_none_or(|value| value.is_null() || value.as_str().is_some_and(str::is_empty)));
    if !report_only {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if changes.is_empty() && input != output {
            std::fs::copy(input, output).map_err(|error| error.to_string())?;
        } else if !changes.is_empty() {
            nif.save(Some(output.to_path_buf()))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(json!({
        "processor": processor,
        "path": input,
        "output": output,
        "game": crate::validation::nif_game_label(&nif),
        "report_only": report_only,
        "changed": !changes.is_empty(),
        "changes": changes,
    }))
}

fn update_mopp_code(nif: &mut NifFile) -> Result<Vec<String>, String> {
    let mopp_ids = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkMoppBvTreeShape")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();

    for mopp_id in mopp_ids {
        let Some(packed_id) = value_ref(nif.blocks[mopp_id].get_field("Shape"))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
            .filter(|reference| *reference < nif.blocks.len())
        else {
            continue;
        };
        if nif.blocks[packed_id].type_name != "bhkPackedNiTriStripsShape" {
            continue;
        }
        let Some(data_id) = value_ref(nif.blocks[packed_id].get_field("Data"))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
            .filter(|reference| *reference < nif.blocks.len())
        else {
            continue;
        };
        if nif.blocks[data_id].type_name != "hkPackedNiTriStripsData" {
            continue;
        }

        let subshapes = nif.blocks[packed_id]
            .get_field("Sub Shapes")
            .or_else(|| nif.blocks[data_id].get_field("Sub Shapes"));
        let Some(subshapes) = subshapes else {
            continue;
        };
        let subshape_vertex_counts = value_array(Some(subshapes))
            .iter()
            .map(|entry| nested_u64(Some(entry), "Num Vertices").unwrap_or(0) as usize)
            .collect::<Vec<_>>();

        let vertices = value_array(nif.blocks[data_id].get_field("Vertices"))
            .iter()
            .map(|vertex| {
                vec3_value(vertex).ok_or_else(|| {
                    format!("hkPackedNiTriStripsData {data_id} has an invalid vertex")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let triangles = value_array(nif.blocks[data_id].get_field("Triangles"))
            .iter()
            .map(|entry| {
                triangle_value(nested_value(Some(entry), "Triangle").unwrap_or(entry)).ok_or_else(
                    || format!("hkPackedNiTriStripsData {data_id} has an invalid triangle"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if vertices.is_empty() || triangles.is_empty() {
            continue;
        }
        let radius = value_f64(nif.blocks[packed_id].get_field("Radius")).unwrap_or(0.1) as f32;
        let (data, origin, scale) = crate::mopp::compile_mopp(&vertices, &triangles, radius)?;
        if data.is_empty() {
            return Err(format!(
                "MOPP generator returned 0 bytes for block {mopp_id}"
            ));
        }

        let block = &mut nif.blocks[mopp_id];
        block.set_field("Scale", NifValue::Float(scale as f64));
        let mopp_code = block
            .get_field_mut("MOPP Code")
            .ok_or_else(|| format!("bhkMoppBvTreeShape {mopp_id} has no MOPP Code field"))?;
        let NifValue::Struct(fields) = mopp_code else {
            return Err(format!(
                "bhkMoppBvTreeShape {mopp_id} has an invalid MOPP Code field"
            ));
        };
        if let Some(value) = named_value_mut(fields, "Data Size") {
            *value = NifValue::UInt(data.len() as u64);
        } else {
            fields.insert("Data Size".to_string(), NifValue::UInt(data.len() as u64));
        }
        if let Some(value) = named_value_mut(fields, "Offset") {
            *value = NifValue::Vec4([origin[0], origin[1], origin[2], scale]);
        } else {
            fields.insert(
                "Offset".to_string(),
                NifValue::Vec4([origin[0], origin[1], origin[2], scale]),
            );
        }
        if let Some(value) = named_value_mut(fields, "Data") {
            *value = NifValue::Bytes(data);
        } else {
            fields.insert("Data".to_string(), NifValue::Bytes(data));
        }
        changes.push(format!(
            "{mopp_id} bhkMoppBvTreeShape: Updated MOPP code from {} triangles across {} subshapes",
            triangles.len(),
            subshape_vertex_counts.len()
        ));
    }

    Ok(changes)
}

fn optimize_mesh(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let triangulate = option_bool(options, "triangulate").unwrap_or(false);
    let stripify = option_bool(options, "stripify").unwrap_or(false);
    let vertex_cache = option_bool(options, "vertex_cache").unwrap_or(true);
    let overdraw = option_bool(options, "overdraw").unwrap_or(true);
    let vertex_fetch = option_bool(options, "vertex_fetch").unwrap_or(true);
    if triangulate && stripify {
        return Err("Triangulate and stripify are mutually exclusive".to_string());
    }
    if !triangulate && !stripify && !vertex_cache && !overdraw && !vertex_fetch {
        return Err("Select at least one optimization".to_string());
    }
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom")
                || SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut processed_data = HashSet::new();
    let mut changes = Vec::new();

    for shape_id in shape_ids {
        let modern = SCHEMA.is_subtype_of(&nif.blocks[shape_id].type_name, "BSTriShape");
        let data_id = if modern {
            shape_id
        } else {
            let Some(reference) = value_ref(nif.blocks[shape_id].get_field("Data"))
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize)
                .filter(|reference| *reference < nif.blocks.len())
            else {
                continue;
            };
            reference
        };
        if !processed_data.insert(data_id) {
            continue;
        }
        if triangulate && !modern {
            let was_strip = nif.blocks[shape_id].type_name == "NiTriStrips"
                || nif.blocks[data_id].type_name == "NiTriStripsData";
            triangulate_legacy_shape(nif, shape_id)?;
            if was_strip {
                changes.push(format!("{shape_id}: Triangulated mesh"));
            }
        }

        let mut indices = triangles(&nif.blocks[data_id])
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let positions = positions(&nif.blocks[data_id]);
        if indices.is_empty() || positions.is_empty() {
            continue;
        }
        let original_indices = indices.clone();
        if vertex_cache {
            indices = meshopt::optimize_vertex_cache(&indices, positions.len());
        }
        if overdraw {
            meshopt::optimize_overdraw_in_place_decoder(&mut indices, &positions, 1.05);
        }
        if vertex_fetch {
            let remap = vertex_fetch_order(&indices, positions.len());
            for index in &mut indices {
                if let Some(replacement) = remap.get(*index as usize) {
                    *index = *replacement;
                }
            }
            reorder_vertex_data(&mut nif.blocks[data_id], modern, &remap);
            remap_skin_vertex_indices(nif, shape_id, &remap);
        }
        if indices != original_indices || vertex_fetch {
            let triangles = indices
                .chunks_exact(3)
                .map(|triangle| [triangle[0], triangle[1], triangle[2]])
                .collect::<Vec<_>>();
            set_triangles(&mut nif.blocks[data_id], &triangles);
            set_geometry_counts(&mut nif.blocks[data_id]);
            changes.push(format!("{shape_id}: Optimized triangle and vertex order"));
        }

        if stripify && !modern && nif.blocks[shape_id].type_name == "NiTriShape" {
            let strips = meshopt::stripify(&indices, positions.len(), 0)
                .map_err(|error| error.to_string())?;
            nif.convert_block_type(shape_id, "NiTriStrips")?;
            nif.convert_block_type(data_id, "NiTriStripsData")?;
            nif.blocks[data_id].set_field("Num Strips", NifValue::UInt(1));
            nif.blocks[data_id].set_field(
                "Strip Lengths",
                NifValue::Array(vec![NifValue::UInt(strips.len() as u64)]),
            );
            nif.blocks[data_id].set_field("Has Points", NifValue::Bool(true));
            nif.blocks[data_id].set_field(
                "Points",
                NifValue::Array(vec![NifValue::Array(
                    strips
                        .into_iter()
                        .map(|index| NifValue::UInt(index as u64))
                        .collect(),
                )]),
            );
            changes.push(format!("{shape_id}: Stripified mesh"));
        }
    }
    Ok(changes)
}

fn vertex_fetch_order(indices: &[u32], vertex_count: usize) -> Vec<u32> {
    let mut remap = vec![u32::MAX; vertex_count];
    let mut next = 0u32;
    for index in indices {
        if let Some(mapped) = remap.get_mut(*index as usize) {
            if *mapped == u32::MAX {
                *mapped = next;
                next += 1;
            }
        }
    }
    for mapped in &mut remap {
        if *mapped == u32::MAX {
            *mapped = next;
            next += 1;
        }
    }
    remap
}

fn reorder_vertex_data(block: &mut NifBlock, modern: bool, remap: &[u32]) {
    if modern {
        reorder_array_field(block, "Vertex Data", remap);
        return;
    }
    for field in [
        "Vertices",
        "Normals",
        "Tangents",
        "Bitangents",
        "Vertex Colors",
    ] {
        reorder_array_field(block, field, remap);
    }
    if let Some(NifValue::Array(sets)) = block.get_field_mut("UV Sets") {
        for set in sets {
            if let NifValue::Array(values) = set {
                reorder_values(values, remap);
            }
        }
    }
}

fn reorder_array_field(block: &mut NifBlock, field: &str, remap: &[u32]) {
    if let Some(NifValue::Array(values)) = block.get_field_mut(field) {
        reorder_values(values, remap);
    }
}

fn reorder_values(values: &mut Vec<NifValue>, remap: &[u32]) {
    if values.len() != remap.len() {
        return;
    }
    let original = values.clone();
    for (old_index, new_index) in remap.iter().copied().enumerate() {
        if let Some(destination) = values.get_mut(new_index as usize) {
            *destination = original[old_index].clone();
        }
    }
}

fn remap_skin_vertex_indices(nif: &mut NifFile, shape_id: usize, remap: &[u32]) {
    let skin_id = ["Skin Instance", "Skin"]
        .iter()
        .find_map(|field| value_ref(nif.blocks[shape_id].get_field(field)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize);
    let data_id = skin_id
        .and_then(|skin_id| nif.blocks.get(skin_id))
        .and_then(|skin| value_ref(skin.get_field("Data")))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize);
    let Some(data_id) = data_id.filter(|data_id| *data_id < nif.blocks.len()) else {
        return;
    };
    let Some(NifValue::Array(bones)) = nif.blocks[data_id].get_field_mut("Bone List") else {
        return;
    };
    for bone in bones {
        let Some(NifValue::Array(weights)) = nested_value_mut(Some(bone), "Vertex Weights") else {
            continue;
        };
        for weight in weights {
            let Some(index) = nested_value_mut(Some(weight), "Index") else {
                continue;
            };
            let Some(old_index) = value_u64(Some(index)).map(|value| value as usize) else {
                continue;
            };
            if let Some(new_index) = remap.get(old_index) {
                *index = NifValue::UInt(*new_index as u64);
            }
        }
    }
}

fn update_tangents(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let add_if_missing = option_bool(options, "add_if_missing").unwrap_or(false);
    let face_normals = option_bool(options, "face_normals").unwrap_or(false);
    let is_oblivion = crate::validation::nif_game_label(nif) == "oblivion";
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
                || SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom")
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();

    for shape_id in shape_ids {
        if geometry_shader_id(nif, shape_id).is_some_and(|shader_id| {
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_MODEL_SPACE_NORMALS != 0)
        }) {
            continue;
        }
        let modern = SCHEMA.is_subtype_of(&nif.blocks[shape_id].type_name, "BSTriShape");
        let data_id = if modern {
            skyrim_skin_partition(nif, shape_id).unwrap_or(shape_id)
        } else {
            value_ref(nif.blocks[shape_id].get_field("Data"))
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize)
                .filter(|reference| *reference < nif.blocks.len())
                .unwrap_or(usize::MAX)
        };
        if data_id == usize::MAX {
            continue;
        }

        let mut geometry = tangent_geometry(&nif.blocks[data_id]);
        if geometry.positions.is_empty()
            || geometry.uvs.len() != geometry.positions.len()
            || geometry.triangles.is_empty()
        {
            continue;
        }
        if face_normals {
            geometry.normals = recompute_face_normals(&geometry.positions, &geometry.triangles);
            if geometry.normals.len() == geometry.positions.len() {
                write_normals(&mut nif.blocks[data_id], modern, &geometry.normals);
                changes.push(format!("{shape_id}: Recalculated normals"));
            }
        }
        if geometry.normals.len() != geometry.positions.len() {
            continue;
        }

        let has_tangents = if modern {
            vertex_desc_flags(&nif.blocks[data_id]) & VF_TANGENTS != 0
        } else if is_oblivion {
            oblivion_tangent_block(nif, shape_id).is_some()
        } else {
            value_u64(nif.blocks[data_id].get_field("Data Flags"))
                .or_else(|| value_u64(nif.blocks[data_id].get_field("BS Data Flags")))
                .is_some_and(|flags| flags & LEGACY_HAS_TANGENTS != 0)
                || !value_array(nif.blocks[data_id].get_field("Tangents")).is_empty()
        };
        if !has_tangents && !add_if_missing {
            continue;
        }

        let (tangents, bitangents) = recompute_tangents_lengyel(
            &geometry.positions,
            &geometry.normals,
            &geometry.uvs,
            &geometry.triangles,
        );
        if tangents.is_empty() {
            continue;
        }
        if modern {
            write_modern_tangents(&mut nif.blocks[data_id], &tangents, &bitangents);
        } else if is_oblivion {
            write_oblivion_tangents(nif, shape_id, &tangents, &bitangents);
        } else {
            write_legacy_tangents(&mut nif.blocks[data_id], &tangents, &bitangents);
        }
        changes.push(format!("{shape_id}: Updated tangents and binormals"));
    }
    changes
}

#[derive(Default)]
struct TangentGeometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    triangles: Vec<[u32; 3]>,
}

fn tangent_geometry(block: &NifBlock) -> TangentGeometry {
    let vertex_data = value_array(block.get_field("Vertex Data"));
    let positions = if vertex_data.is_empty() {
        value_array(block.get_field("Vertices"))
            .iter()
            .filter_map(vec3_value)
            .collect()
    } else {
        vertex_data
            .iter()
            .filter_map(|entry| nested_value(Some(entry), "Vertex").and_then(vec3_value))
            .collect()
    };
    let normals = if vertex_data.is_empty() {
        value_array(block.get_field("Normals"))
            .iter()
            .filter_map(vec3_value)
            .collect()
    } else {
        vertex_data
            .iter()
            .filter_map(|entry| nested_value(Some(entry), "Normal").and_then(vec3_value))
            .collect()
    };
    let uvs = if vertex_data.is_empty() {
        value_array(block.get_field("UV Sets"))
            .first()
            .map(|set| value_array(Some(set)).iter().filter_map(uv_value).collect())
            .unwrap_or_default()
    } else {
        vertex_data
            .iter()
            .filter_map(|entry| nested_value(Some(entry), "UV").and_then(uv_value))
            .collect()
    };
    TangentGeometry {
        positions,
        normals,
        uvs,
        triangles: triangles(block),
    }
}

fn triangles(block: &NifBlock) -> Vec<[u32; 3]> {
    let direct = value_array(block.get_field("Triangles"))
        .iter()
        .filter_map(triangle_value)
        .collect::<Vec<_>>();
    if !direct.is_empty() {
        return direct;
    }
    let strips = if block.get_field("Points").is_some() {
        value_array(block.get_field("Points"))
    } else {
        value_array(block.get_field("Strips"))
    };
    if !strips.is_empty() {
        return strip_triangles(strips);
    }
    value_array(block.get_field("Partitions"))
        .iter()
        .flat_map(|partition| {
            nested_array(Some(partition), "Triangles")
                .iter()
                .filter_map(triangle_value)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn strip_triangles(strips: &[NifValue]) -> Vec<[u32; 3]> {
    strips
        .iter()
        .flat_map(|strip| {
            value_array(Some(strip))
                .iter()
                .filter_map(|value| value_u64(Some(value)))
                .filter_map(|value| u32::try_from(value).ok())
                .collect::<Vec<_>>()
                .windows(3)
                .enumerate()
                .filter_map(|(index, triangle)| {
                    let [a, b, c] = <[u32; 3]>::try_from(triangle).ok()?;
                    (a != b && b != c && a != c).then_some(if index % 2 == 0 {
                        [a, b, c]
                    } else {
                        [b, a, c]
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn recompute_face_normals(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0; 3]; positions.len()];
    for triangle in triangles {
        let [a, b, c] = triangle.map(|index| index as usize);
        let (Some(a), Some(b), Some(c)) = (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        for index in triangle.map(|index| index as usize) {
            if let Some(normal) = normals.get_mut(index) {
                for axis in 0..3 {
                    normal[axis] += face[axis];
                }
            }
        }
    }
    for normal in &mut normals {
        let length = normal
            .iter()
            .map(|component| component * component)
            .sum::<f32>()
            .sqrt();
        if length > 1e-8 {
            for component in normal {
                *component /= length;
            }
        }
    }
    normals
}

fn write_normals(block: &mut NifBlock, modern: bool, normals: &[[f32; 3]]) {
    if modern {
        let desc = value_u64(block.get_field("Vertex Desc")).unwrap_or(0);
        block.set_field(
            "Vertex Desc",
            NifValue::UInt(insert_vertex_attribute(
                desc,
                VF_NORMALS,
                16,
                &[20, 24, 28, 32, 36],
            )),
        );
        if let Some(NifValue::Array(entries)) = block.get_field_mut("Vertex Data") {
            for (entry, normal) in entries.iter_mut().zip(normals) {
                if let NifValue::Struct(fields) = entry {
                    fields.insert("Normal".to_string(), NifValue::Vec3(*normal));
                }
            }
        }
    } else {
        block.set_field("Has Normals", NifValue::Bool(true));
        block.set_field(
            "Normals",
            NifValue::Array(normals.iter().copied().map(NifValue::Vec3).collect()),
        );
    }
}

fn write_modern_tangents(block: &mut NifBlock, tangents: &[[f32; 3]], bitangents: &[[f32; 3]]) {
    let desc = value_u64(block.get_field("Vertex Desc")).unwrap_or(0);
    block.set_field(
        "Vertex Desc",
        NifValue::UInt(insert_vertex_attribute(
            desc,
            VF_TANGENTS,
            20,
            &[24, 28, 32, 36],
        )),
    );
    if let Some(NifValue::Array(entries)) = block.get_field_mut("Vertex Data") {
        for ((entry, tangent), bitangent) in entries.iter_mut().zip(tangents).zip(bitangents) {
            if let NifValue::Struct(fields) = entry {
                fields.insert(
                    "Bitangent X".to_string(),
                    NifValue::Float(bitangent[0] as f64),
                );
                fields.insert(
                    "Bitangent Y".to_string(),
                    NifValue::Float(bitangent[1] as f64),
                );
                fields.insert("Tangent".to_string(), NifValue::Vec3(*tangent));
                fields.insert(
                    "Bitangent Z".to_string(),
                    NifValue::Float(bitangent[2] as f64),
                );
            }
        }
    }
}

fn insert_vertex_attribute(
    descriptor: u64,
    attribute: u64,
    offset_shift: u32,
    following_offsets: &[u32],
) -> u64 {
    let flags = descriptor >> 44;
    if flags & attribute != 0 {
        return descriptor;
    }
    let stride = descriptor & 0xf;
    let insertion = following_offsets
        .iter()
        .map(|shift| (descriptor >> shift) & 0xf)
        .find(|offset| *offset != 0)
        .unwrap_or(stride);
    let mut updated = (descriptor & !0xf) | ((stride + 1).min(0xf));
    for shift in following_offsets {
        let mask = 0xfu64 << shift;
        let offset = (updated & mask) >> shift;
        if offset >= insertion && offset != 0 {
            updated = (updated & !mask) | (((offset + 1).min(0xf)) << shift);
        }
    }
    let offset_mask = 0xfu64 << offset_shift;
    updated = (updated & !offset_mask) | (insertion << offset_shift);
    updated | (attribute << 44)
}

fn write_legacy_tangents(block: &mut NifBlock, tangents: &[[f32; 3]], bitangents: &[[f32; 3]]) {
    let flag_field = if block.get_field("Data Flags").is_some() {
        "Data Flags"
    } else {
        "BS Data Flags"
    };
    let flags = value_u64(block.get_field(flag_field)).unwrap_or(0) | LEGACY_HAS_TANGENTS;
    block.set_field(flag_field, NifValue::UInt(flags));
    block.set_field(
        "Tangents",
        NifValue::Array(tangents.iter().copied().map(NifValue::Vec3).collect()),
    );
    block.set_field(
        "Bitangents",
        NifValue::Array(bitangents.iter().copied().map(NifValue::Vec3).collect()),
    );
}

fn oblivion_tangent_block(nif: &NifFile, shape_id: usize) -> Option<usize> {
    value_array(nif.blocks[shape_id].get_field("Extra Data List"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .find(|reference| {
            nif.get_block(*reference)
                .filter(|block| block.type_name == "NiBinaryExtraData")
                .and_then(|block| block.get_field("Name"))
                .and_then(value_string)
                .is_some_and(|name| name == OBLIVION_TANGENT_DATA_NAME)
        })
}

fn write_oblivion_tangents(
    nif: &mut NifFile,
    shape_id: usize,
    tangents: &[[f32; 3]],
    bitangents: &[[f32; 3]],
) {
    let mut bytes = Vec::with_capacity((tangents.len() + bitangents.len()) * 12);
    for vector in tangents.iter().chain(bitangents) {
        for component in vector {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    let extra_id = oblivion_tangent_block(nif, shape_id).unwrap_or_else(|| {
        let mut fields = IndexMap::new();
        fields.insert(
            "Name".to_string(),
            NifValue::String(OBLIVION_TANGENT_DATA_NAME.to_string()),
        );
        fields.insert(
            "Binary Data".to_string(),
            NifValue::Struct(IndexMap::from([
                ("Data Size".to_string(), NifValue::UInt(0)),
                ("Data".to_string(), NifValue::Bytes(Vec::new())),
            ])),
        );
        let extra_id = nif.add_block("NiBinaryExtraData", Some(fields));
        let refs = match nif.blocks[shape_id].get_field("Extra Data List") {
            Some(NifValue::Array(refs)) => refs.clone(),
            _ => Vec::new(),
        };
        let mut refs = refs;
        refs.push(NifValue::Ref(extra_id as i32));
        nif.blocks[shape_id].set_field("Num Extra Data List", NifValue::UInt(refs.len() as u64));
        nif.blocks[shape_id].set_field("Extra Data List", NifValue::Array(refs));
        extra_id
    });
    nif.blocks[extra_id].set_field(
        "Binary Data",
        NifValue::Struct(IndexMap::from([
            ("Data Size".to_string(), NifValue::UInt(bytes.len() as u64)),
            ("Data".to_string(), NifValue::Bytes(bytes)),
        ])),
    );
}

fn skyrim_skin_partition(nif: &NifFile, shape_id: usize) -> Option<usize> {
    if crate::validation::nif_game_label(nif) != "skyrimse" {
        return None;
    }
    let skin_id = value_ref(nif.blocks[shape_id].get_field("Skin"))?;
    let skin = nif.get_block(usize::try_from(skin_id).ok()?)?;
    let partition_id = value_ref(skin.get_field("Skin Partition"))?;
    usize::try_from(partition_id).ok()
}

fn vertex_desc_flags(block: &NifBlock) -> u64 {
    value_u64(block.get_field("Vertex Desc")).unwrap_or(0) >> 44
}

fn update_bounds(nif: &mut NifFile) -> Vec<String> {
    let updates = nif
        .blocks
        .iter()
        .filter_map(|block| {
            let positions = positions(block);
            (!positions.is_empty()).then(|| (block.block_id, bounding_sphere(&positions)))
        })
        .collect::<Vec<_>>();
    let mut changed = Vec::new();
    for (block_id, sphere) in updates {
        let block = &mut nif.blocks[block_id];
        if block.get_field("Bounding Sphere") != Some(&sphere) {
            block.set_field("Bounding Sphere", sphere);
            changed.push(format!(
                "{} {}: Updated bounds",
                block.block_id, block.type_name
            ));
        }
    }
    changed
}

fn update_shader_flags(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let flags1 = option_u64(options, "flags1").unwrap_or(0);
    let flags2 = option_u64(options, "flags2").unwrap_or(0);
    if flags1 == 0 && flags2 == 0 {
        return Err("No flags selected".to_string());
    }
    let mode = options.get("mode").and_then(Value::as_str).unwrap_or("add");
    if !matches!(mode, "add" | "set" | "remove") {
        return Err(format!("Invalid shader flag mode: {mode}"));
    }
    let flags1_field = if crate::validation::nif_game_label(nif) == "fo3/fnv" {
        "Shader Flags"
    } else {
        "Shader Flags 1"
    };
    let mut changes = Vec::new();
    for block in &mut nif.blocks {
        if !SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty") {
            continue;
        }
        for (field, selected) in [(flags1_field, flags1), ("Shader Flags 2", flags2)] {
            if selected == 0 || block.get_field(field).is_none() {
                continue;
            }
            let old = value_u64(block.get_field(field)).unwrap_or(0);
            let new = match mode {
                "add" => old | selected,
                "set" => selected,
                "remove" => old & !selected,
                _ => unreachable!(),
            };
            if old != new {
                block.set_field(field, NifValue::UInt(new));
                changes.push(format!(
                    "{} {field}: Updated flags from {old:#010x} to {new:#010x}",
                    block.block_id
                ));
            }
        }
    }
    Ok(changes)
}

fn walls_reflection_flag(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let map_scale = option_f64(options, "map_scale");
    let normal_intensity = option_f64(options, "normal_intensity");
    let blend_intensity = option_f64(options, "blend_intensity");
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom"))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    for shape_id in shape_ids {
        let Some(shader_id) = property_by_type(nif, shape_id, "BSShaderPPLightingProperty") else {
            continue;
        };
        let flags1 = value_u64(nif.blocks[shader_id].get_field("Shader Flags")).unwrap_or(0);
        if flags1 & (1 << 7) == 0 {
            continue;
        }
        let old_flags2 = value_u64(nif.blocks[shader_id].get_field("Shader Flags 2")).unwrap_or(0);
        let new_flags2 = (old_flags2 & !(1 << 15)) | (1 << 31);
        if old_flags2 != new_flags2 {
            nif.blocks[shader_id].set_field("Shader Flags 2", NifValue::UInt(new_flags2));
            changes.push(format!("{shader_id}: Enabled real-time reflections"));
        }
        if let Some(scale) = map_scale {
            let old = value_f64(nif.blocks[shader_id].get_field("Environment Map Scale"));
            if old.is_none_or(|old| (old - scale).abs() > f64::EPSILON) {
                nif.blocks[shader_id].set_field("Environment Map Scale", NifValue::Float(scale));
                changes.push(format!("{shader_id}: Set environment map scale to {scale}"));
            }
        }
        for (name, value) in [
            ("NormalIntensity", normal_intensity),
            ("BlendIntensity", blend_intensity),
        ] {
            if let Some(value) = value {
                if set_float_extra_data(nif, shape_id, name, value) {
                    changes.push(format!("{shape_id}: Set {name} to {value}"));
                }
            }
        }
    }
    changes
}

fn soft_particles(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let soft_scale = option_f64(options, "soft_scale");
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom"))
        .filter(|block| {
            !block
                .get_field("Name")
                .and_then(value_string)
                .is_some_and(|name| name.to_ascii_lowercase().contains("editormarker"))
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    for shape_id in shape_ids {
        let Some(shader_id) = property_by_type(nif, shape_id, "BSShaderNoLightingProperty") else {
            continue;
        };
        let flags2 = value_u64(nif.blocks[shader_id].get_field("Shader Flags 2")).unwrap_or(0);
        if flags2 & (1 << 30) == 0 {
            nif.blocks[shader_id].set_field("Shader Flags 2", NifValue::UInt(flags2 | (1 << 30)));
            changes.push(format!("{shader_id}: Enabled soft particles"));
        }
        if let Some(scale) = soft_scale {
            if set_float_extra_data(nif, shape_id, "VPSoftScale", scale) {
                changes.push(format!("{shape_id}: Set VPSoftScale to {scale}"));
            }
        }
    }
    changes
}

fn update_ragdoll_constraints(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    if option_bool(options, "convert_to_malleable").unwrap_or(false) {
        let conversions = nif
            .blocks
            .iter()
            .filter_map(|block| {
                let (constraint_type, field) = match block.type_name.as_str() {
                    "bhkBallAndSocketConstraint" => (0, "Ball and Socket"),
                    "bhkHingeConstraint" => (1, "Hinge"),
                    "bhkLimitedHingeConstraint" => (2, "Limited Hinge"),
                    "bhkPrismaticConstraint" => (6, "Prismatic"),
                    "bhkRagdollConstraint" => (7, "Ragdoll"),
                    "bhkStiffSpringConstraint" => (8, "Stiff Spring"),
                    _ => return None,
                };
                Some((
                    block.block_id,
                    block.type_name.clone(),
                    constraint_type,
                    field,
                    block.get_field("Constraint").cloned(),
                ))
            })
            .collect::<Vec<_>>();
        for (block_id, old_type, constraint_type, field, old_constraint) in conversions {
            if !nif.convert_block_type(block_id, "bhkMalleableConstraint")? {
                continue;
            }
            let mut malleable = match nif.blocks[block_id].get_field("Constraint").cloned() {
                Some(NifValue::Struct(fields)) => fields,
                _ => IndexMap::new(),
            };
            malleable.insert("Type".to_string(), NifValue::UInt(constraint_type));
            if let Some(old_constraint) = old_constraint {
                malleable.insert(field.to_string(), old_constraint);
            }
            nif.blocks[block_id].set_field("Constraint", NifValue::Struct(malleable));
            changes.push(format!(
                "{block_id}: Converted {old_type} to bhkMalleableConstraint"
            ));
        }
    }
    for block in &mut nif.blocks {
        let ragdoll = if block.type_name == "bhkRagdollConstraint" {
            block.get_field("Constraint")
        } else if block.type_name == "bhkMalleableConstraint" {
            let constraint = block.get_field("Constraint");
            if nested_u64(constraint, "Type") != Some(7) {
                continue;
            }
            nested_value(constraint, "Ragdoll")
        } else {
            continue;
        };
        let Some(ragdoll) = ragdoll.cloned() else {
            continue;
        };
        let mut updates = Vec::new();
        for side in ["A", "B"] {
            let twist = nested_value(Some(&ragdoll), &format!("Twist {side}")).and_then(vec4_xyz);
            let plane = nested_value(Some(&ragdoll), &format!("Plane {side}")).and_then(vec4_xyz);
            let motor = nested_value(Some(&ragdoll), &format!("Motor {side}")).and_then(vec4_xyz);
            let (Some(twist), Some(plane), Some(motor)) = (twist, plane, motor) else {
                continue;
            };
            let expected = [
                twist[1] * plane[2] - twist[2] * plane[1],
                twist[2] * plane[0] - twist[0] * plane[2],
                twist[0] * plane[1] - twist[1] * plane[0],
            ];
            if motor
                .iter()
                .zip(expected)
                .any(|(old, new)| (*old - new).abs() > f32::EPSILON)
            {
                updates.push((format!("Motor {side}"), expected));
            }
        }
        if updates.is_empty() {
            continue;
        }
        let block_id = block.block_id;
        let is_malleable = block.type_name == "bhkMalleableConstraint";
        let constraint = block.get_field_mut("Constraint").unwrap();
        let ragdoll = if is_malleable {
            nested_value_mut(Some(constraint), "Ragdoll").unwrap()
        } else {
            constraint
        };
        for (field, expected) in updates {
            if let Some(motor) = nested_value_mut(Some(ragdoll), &field) {
                set_vec4_xyz(motor, expected);
                changes.push(format!("{block_id} {field}: Updated ragdoll motor"));
            }
        }
    }
    Ok(changes)
}

fn update_havok_settings(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let empty_settings = serde_json::Map::new();
    let settings = options
        .get("settings")
        .and_then(Value::as_object)
        .unwrap_or(&empty_settings);
    let game = crate::validation::nif_game_label(nif);
    let material_type = match game {
        "morrowind" | "oblivion" => "OblivionHavokMaterial",
        "fo3/fnv" => "Fallout3HavokMaterial",
        "skyrim" | "skyrimse" => "SkyrimHavokMaterial",
        "fo4" => "Fallout4HavokMaterial",
        "fo76" | "starfield" => "Fallout76HavokMaterial",
        _ => "SkyrimHavokMaterial",
    };
    let layer_type = match game {
        "morrowind" | "oblivion" => "OblivionLayer",
        "fo3/fnv" => "Fallout3Layer",
        "skyrim" | "skyrimse" => "SkyrimLayer",
        "fo4" => "Fallout4Layer",
        "fo76" | "starfield" => "Fallout76Layer",
        _ => "SkyrimLayer",
    };
    let material = setting_u64(settings.get("material"), &[material_type])?;
    let layer = setting_u64(settings.get("layer"), &[layer_type])?;
    let radius = json_f64(settings.get("radius"));
    let rigid_values = [
        ("Mass", json_f64(settings.get("mass"))),
        ("Linear Damping", json_f64(settings.get("linear_damping"))),
        ("Angular Damping", json_f64(settings.get("angular_damping"))),
        (
            "Max Linear Velocity",
            json_f64(settings.get("max_linear_velocity")),
        ),
        (
            "Max Angular Velocity",
            json_f64(settings.get("max_angular_velocity")),
        ),
        ("Friction", json_f64(settings.get("friction"))),
        ("Restitution", json_f64(settings.get("restitution"))),
    ];
    let rigid_enums = [
        (
            "Motion System",
            setting_u64(settings.get("motion_system"), &["hkMotionType"])?,
        ),
        (
            "Deactivator Type",
            setting_u64(settings.get("deactivator_type"), &["hkDeactivatorType"])?,
        ),
        (
            "Solver Deactivation",
            setting_u64(
                settings.get("solver_deactivation"),
                &["hkSolverDeactivation"],
            )?,
        ),
        (
            "Motion Quality",
            setting_u64(settings.get("motion_quality"), &["hkQualityType"])?,
        ),
    ];
    let mut changes = Vec::new();
    for block in &mut nif.blocks {
        let block_id = block.block_id;
        if SCHEMA.is_subtype_of(&block.type_name, "bhkShape") {
            if let Some(material) = material {
                record_field_change(
                    block_id,
                    "Material",
                    set_block_field_if_present(block, "Material", NifValue::UInt(material)),
                    &mut changes,
                );
            }
            if let Some(radius) = radius {
                for field in ["Radius", "Radius Copy"] {
                    record_field_change(
                        block_id,
                        field,
                        set_block_field_if_present(block, field, NifValue::Float(radius)),
                        &mut changes,
                    );
                }
            }
            if let Some(layer) = layer {
                if let Some(NifValue::Array(filters)) = block.get_field_mut("Filters") {
                    for filter in filters {
                        if set_struct_field_if_present(filter, "Layer", NifValue::UInt(layer)) {
                            changes.push(format!("{block_id} Filters: Set layer to {layer}"));
                        }
                    }
                }
            }
        } else if matches!(
            block.type_name.as_str(),
            "hkPackedNiTriStripsData" | "bhkCompressedMeshShapeData"
        ) {
            if let Some(material) = material {
                for field in ["Sub Shapes", "Chunk Materials"] {
                    if let Some(NifValue::Array(entries)) = block.get_field_mut(field) {
                        for entry in entries {
                            if set_struct_field_if_present(
                                entry,
                                "Material",
                                NifValue::UInt(material),
                            ) {
                                changes.push(format!(
                                    "{block_id} {field}: Set material to {material}"
                                ));
                            }
                        }
                        break;
                    }
                }
            }
        } else if SCHEMA.is_subtype_of(&block.type_name, "bhkRigidBody") {
            if let Some(layer) = layer {
                for group in ["Havok Filter", "Havok Filter Copy"] {
                    if set_rigid_nested_field_if_present(
                        block,
                        group,
                        "Layer",
                        NifValue::UInt(layer),
                    ) {
                        changes.push(format!("{block_id} {group}: Set layer to {layer}"));
                    }
                }
            }
            for (field, value) in rigid_values {
                if let Some(value) = value {
                    if set_rigid_field_if_present(block, field, NifValue::Float(value)) {
                        changes.push(format!("{block_id} {field}: Set to {value}"));
                    }
                }
            }
            for (field, value) in rigid_enums {
                if let Some(value) = value {
                    if set_rigid_field_if_present(block, field, NifValue::UInt(value)) {
                        changes.push(format!("{block_id} {field}: Set to {value}"));
                    }
                }
            }
            if settings
                .get("mass")
                .is_some_and(|value| json_f64(Some(value)) == Some(0.0))
            {
                set_rigid_field_if_present(
                    block,
                    "Inertia Tensor",
                    NifValue::Matrix33([[0.0; 3]; 3]),
                );
            }
        }
    }
    Ok(changes)
}

fn update_havok_inertia(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let update_inertia = option_bool(options, "update_inertia").unwrap_or(true);
    let update_center = option_bool(options, "update_center").unwrap_or(true);
    let update_penetration = option_bool(options, "update_penetration").unwrap_or(false);
    let penetration_statics = option_bool(options, "penetration_statics").unwrap_or(false);
    if !update_inertia && !update_center && !update_penetration {
        return Err("No update options selected".to_string());
    }
    let depth_multiplier = option_f64(options, "depth_multiplier")
        .filter(|value| *value != 0.0)
        .unwrap_or(0.2);
    let mut body_multipliers = IndexMap::from([
        (1u64, 2.0f64),
        (2, 3.0),
        (3, 3.0),
        (4, 3.0),
        (5, 2.0),
        (8, 2.0),
        (11, 2.0),
        (14, 2.0),
    ]);
    if let Some(values) = options
        .get("body_part_multipliers")
        .and_then(Value::as_object)
    {
        body_multipliers.clear();
        for (part, multiplier) in values {
            if let (Ok(part), Some(multiplier)) = (part.parse::<u64>(), json_f64(Some(multiplier)))
            {
                if multiplier >= 0.0 {
                    body_multipliers.insert(part, multiplier);
                }
            }
        }
    }
    let body_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "bhkRigidBody"))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let game = crate::validation::nif_game_label(nif);
    let mut changes = Vec::new();
    for body_id in body_ids {
        let body = &nif.blocks[body_id];
        let Some(mut shape_id) = value_ref(body.get_field("Shape"))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
        else {
            continue;
        };
        let dynamic = rigid_body_is_dynamic_for_inertia(nif, body);
        let mass = rigid_value(body, "Mass")
            .and_then(|value| value_f64(Some(value)))
            .unwrap_or(0.0);
        let body_part = rigid_nested_value(body, "Havok Filter", "Flags and Part Number")
            .and_then(|value| value_u64(Some(value)))
            .unwrap_or(0);
        let multiplier = body_multipliers.get(&body_part).copied().unwrap_or(1.0);
        let mut transform_offset = [0.0f32; 3];
        while nif
            .get_block(shape_id)
            .is_some_and(|shape| SCHEMA.is_subtype_of(&shape.type_name, "bhkTransformShape"))
        {
            let shape = &nif.blocks[shape_id];
            if let Some(offset) = transform_translation(shape.get_field("Transform")) {
                for axis in 0..3 {
                    transform_offset[axis] += offset[axis];
                }
            }
            let Some(next) = value_ref(shape.get_field("Shape"))
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize)
            else {
                break;
            };
            shape_id = next;
        }
        let Some(shape) = nif.get_block(shape_id) else {
            continue;
        };
        let mut inertia = None;
        let mut center = None;
        let mut depth = None;
        match shape.type_name.as_str() {
            "bhkBoxShape" => {
                if let Some([x, y, z]) = shape.get_field("Dimensions").and_then(vec3_value) {
                    inertia = Some([
                        mass * (y as f64 * y as f64 + z as f64 * z as f64) / 12.0 * multiplier,
                        mass * (x as f64 * x as f64 + z as f64 * z as f64) / 12.0 * multiplier,
                        mass * (x as f64 * x as f64 + y as f64 * y as f64) / 12.0 * multiplier,
                    ]);
                    center = Some(transform_offset);
                    depth = Some(f64::from(x.min(y).min(z)) * depth_multiplier);
                }
            }
            "bhkSphereShape" => {
                if let Some(radius) = value_f64(shape.get_field("Radius")) {
                    let moment = 2.0 * mass * radius * radius / 5.0 * multiplier;
                    inertia = Some([moment; 3]);
                    center = Some(transform_offset);
                    depth = Some(2.0 * radius * depth_multiplier);
                }
            }
            "bhkCapsuleShape" => {
                let radius = value_f64(shape.get_field("Radius"));
                let first = shape.get_field("First Point").and_then(vec3_value);
                let second = shape.get_field("Second Point").and_then(vec3_value);
                if let (Some(radius), Some(first), Some(second)) = (radius, first, second) {
                    let lengths = [
                        f64::from((first[0] - second[0]).abs()),
                        f64::from((first[1] - second[1]).abs()),
                        f64::from((first[2] - second[2]).abs()),
                    ];
                    let axis = if lengths[0] >= lengths[1] && lengths[0] >= lengths[2] {
                        0
                    } else if lengths[1] >= lengths[2] {
                        1
                    } else {
                        2
                    };
                    let axial = mass * radius * radius / 2.0;
                    let transverse = mass * radius * radius / 4.0
                        + mass * (lengths[axis] + 2.0 * radius).powi(2) / 12.0;
                    let mut moments = [transverse * multiplier; 3];
                    moments[axis] = axial * multiplier;
                    inertia = Some(moments);
                    center = Some([
                        (first[0] + second[0]) / 2.0 + transform_offset[0],
                        (first[1] + second[1]) / 2.0 + transform_offset[1],
                        (first[2] + second[2]) / 2.0 + transform_offset[2],
                    ]);
                    depth = Some(2.0 * radius * depth_multiplier);
                }
            }
            "bhkConvexVerticesShape" | "bhkMoppBvTreeShape" => {
                let (vertices, packed_mopp) = collision_shape_vertices(nif, shape_id);
                if !vertices.is_empty() {
                    let (minimum, maximum) = min_max_vertices(&vertices);
                    let dimensions = [
                        f64::from(maximum[0] - minimum[0]),
                        f64::from(maximum[1] - minimum[1]),
                        f64::from(maximum[2] - minimum[2]),
                    ];
                    inertia = Some([
                        mass * (dimensions[1].powi(2) + dimensions[2].powi(2)) / 12.0 * multiplier,
                        mass * (dimensions[0].powi(2) + dimensions[2].powi(2)) / 12.0 * multiplier,
                        mass * (dimensions[0].powi(2) + dimensions[1].powi(2)) / 12.0 * multiplier,
                    ]);
                    let collision_center = [
                        (minimum[0] + maximum[0]) / 2.0,
                        (minimum[1] + maximum[1]) / 2.0,
                        (minimum[2] + maximum[2]) / 2.0,
                    ];
                    center = Some([
                        collision_center[0] + transform_offset[0],
                        collision_center[1] + transform_offset[1],
                        collision_center[2] + transform_offset[2],
                    ]);
                    let minimum_radius = vertices
                        .iter()
                        .map(|vertex| {
                            let dx = vertex[0] - collision_center[0];
                            let dy = vertex[1] - collision_center[1];
                            let dz = vertex[2] - collision_center[2];
                            f64::from((dx * dx + dy * dy + dz * dz).sqrt())
                        })
                        .fold(f64::INFINITY, f64::min);
                    let game_units = if packed_mopp {
                        match game {
                            "fo3/fnv" => 6.999125,
                            "skyrim" | "skyrimse" | "fo4" => 69.99125,
                            _ => 1.0,
                        }
                    } else {
                        1.0
                    };
                    depth = Some(2.0 * minimum_radius * depth_multiplier / game_units);
                }
            }
            _ => {}
        }
        let body = &mut nif.blocks[body_id];
        if update_inertia && dynamic {
            if let Some(inertia) = inertia {
                if set_inertia_diagonal(body, inertia) {
                    changes.push(format!("{body_id}: Updated inertia tensor"));
                }
            }
        }
        if update_center && dynamic {
            if let Some(center) = center {
                if set_rigid_vec3(body, "Center", center) {
                    changes.push(format!("{body_id}: Updated collision center"));
                }
            }
        }
        if update_penetration && (dynamic || penetration_statics) {
            if let Some(mut depth) = depth {
                if depth == 0.0 {
                    depth = 0.04;
                }
                if set_rigid_field_if_present(body, "Penetration Depth", NifValue::Float(depth)) {
                    changes.push(format!("{body_id}: Updated penetration depth"));
                }
            }
        }
    }
    Ok(changes)
}

fn rigid_body_is_dynamic_for_inertia(nif: &NifFile, body: &NifBlock) -> bool {
    let motion = rigid_value(body, "Motion System")
        .and_then(|value| value_u64(Some(value)))
        .unwrap_or(0);
    let layer = rigid_nested_value(body, "Havok Filter", "Layer")
        .and_then(|value| value_u64(Some(value)))
        .unwrap_or(0);
    if matches!(motion, 0 | 7) || (motion == 6 && layer != 8) {
        return false;
    }
    if matches!(
        crate::validation::nif_game_label(nif),
        "skyrim" | "skyrimse" | "fo4"
    ) {
        let quality = rigid_value(body, "Motion Quality")
            .and_then(|value| value_u64(Some(value)))
            .unwrap_or(0);
        layer > 2 && !matches!(quality, 0 | 1)
    } else {
        true
    }
}

fn rigid_value<'a>(block: &'a NifBlock, name: &str) -> Option<&'a NifValue> {
    block.get_field(name).or_else(|| {
        block
            .get_field("Rigid Body Info")
            .and_then(|value| nested_value(Some(value), name))
    })
}

fn rigid_nested_value<'a>(block: &'a NifBlock, group: &str, name: &str) -> Option<&'a NifValue> {
    block
        .get_field(group)
        .and_then(|value| nested_value(Some(value), name))
        .or_else(|| {
            block
                .get_field("Rigid Body Info")
                .and_then(|value| nested_value(Some(value), group))
                .and_then(|value| nested_value(Some(value), name))
        })
}

fn transform_translation(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value? {
        NifValue::Matrix44(matrix) => Some([matrix[0][3], matrix[1][3], matrix[2][3]]),
        NifValue::Struct(fields) => Some([
            named_value(fields, "m14").and_then(|value| value_f64(Some(value)))? as f32,
            named_value(fields, "m24").and_then(|value| value_f64(Some(value)))? as f32,
            named_value(fields, "m34").and_then(|value| value_f64(Some(value)))? as f32,
        ]),
        _ => None,
    }
}

fn collision_shape_vertices(nif: &NifFile, shape_id: usize) -> (Vec<[f32; 3]>, bool) {
    let Some(shape) = nif.get_block(shape_id) else {
        return (Vec::new(), false);
    };
    if shape.type_name == "bhkConvexVerticesShape" {
        return (
            value_array(shape.get_field("Vertices"))
                .iter()
                .filter_map(|value| vec3_value(value).or_else(|| vec4_xyz(value)))
                .collect(),
            false,
        );
    }
    let Some(child) = referenced_block(nif, shape.get_field("Shape")) else {
        return (Vec::new(), false);
    };
    let Some(data) = referenced_block(nif, child.get_field("Data")) else {
        return (Vec::new(), false);
    };
    match data.type_name.as_str() {
        "hkPackedNiTriStripsData" => (
            value_array(data.get_field("Vertices"))
                .iter()
                .filter_map(|value| vec3_value(value).or_else(|| vec4_xyz(value)))
                .collect(),
            true,
        ),
        "bhkCompressedMeshShapeData" => crate::skyrim_collision::decode_compressed_mesh_data(data)
            .map(|(vertices, _)| (vertices, false))
            .unwrap_or_default(),
        _ => (Vec::new(), false),
    }
}

fn min_max_vertices(vertices: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in vertices {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex[axis]);
            maximum[axis] = maximum[axis].max(vertex[axis]);
        }
    }
    (minimum, maximum)
}

fn set_inertia_diagonal(block: &mut NifBlock, diagonal: [f64; 3]) -> bool {
    let Some(mut inertia) = rigid_value(block, "Inertia Tensor").cloned() else {
        return false;
    };
    let before = inertia.clone();
    match &mut inertia {
        NifValue::Matrix33(matrix) => {
            for axis in 0..3 {
                matrix[axis][axis] = diagonal[axis] as f32;
            }
        }
        NifValue::Struct(fields) => {
            for (name, value) in ["m11", "m22", "m33"].into_iter().zip(diagonal) {
                if let Some(field) = named_value_mut(fields, name) {
                    *field = NifValue::Float(value);
                }
            }
        }
        _ => return false,
    }
    before != inertia && set_rigid_field_if_present(block, "Inertia Tensor", inertia)
}

fn set_rigid_vec3(block: &mut NifBlock, field: &str, value: [f32; 3]) -> bool {
    let Some(mut current) = rigid_value(block, field).cloned() else {
        return false;
    };
    let before = current.clone();
    match &mut current {
        NifValue::Vec3(current) | NifValue::Color3(current) => *current = value,
        NifValue::Vec4(current) | NifValue::Quaternion(current) => {
            current[..3].copy_from_slice(&value)
        }
        NifValue::Struct(fields) => {
            for (name, component) in ["x", "y", "z"].into_iter().zip(value) {
                if let Some(field) = named_value_mut(fields, name) {
                    *field = NifValue::Float(component as f64);
                }
            }
        }
        _ => return false,
    }
    before != current && set_rigid_field_if_present(block, field, current)
}

fn search_havok_material(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let material_type = match crate::validation::nif_game_label(nif) {
        "oblivion" => "OblivionHavokMaterial",
        "fo3/fnv" => "Fallout3HavokMaterial",
        "skyrim" | "skyrimse" => "SkyrimHavokMaterial",
        "fo4" => "Fallout4HavokMaterial",
        "fo76" | "starfield" => "Fallout76HavokMaterial",
        _ => "SkyrimHavokMaterial",
    };
    let search = setting_u64(options.get("material_search"), &[material_type])?;
    let replacement = setting_u64(options.get("material_replace"), &[material_type])?;
    if search.is_some() && search == replacement {
        return Err("Searched and replacing materials must be different".to_string());
    }
    let skipped = if option_bool(options, "skip_root").unwrap_or(false) {
        let root = roots(nif).first().copied();
        nif.blocks
            .iter()
            .filter(|block| collision_target(nif, block.block_id) == root)
            .map(|block| block.block_id)
            .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let mut changes = Vec::new();
    for block in &mut nif.blocks {
        let block_id = block.block_id;
        if skipped.contains(&block_id) {
            continue;
        }
        if matches!(
            block.type_name.as_str(),
            "hkPackedNiTriStripsData" | "bhkCompressedMeshShapeData"
        ) {
            for field in ["Sub Shapes", "Chunk Materials"] {
                let Some(NifValue::Array(entries)) = block.get_field_mut(field) else {
                    continue;
                };
                for (index, entry) in entries.iter_mut().enumerate() {
                    let Some(material) = nested_u64(Some(entry), "Material") else {
                        continue;
                    };
                    if search.is_some_and(|search| search != material) {
                        continue;
                    }
                    if let Some(replacement) = replacement {
                        if set_struct_field_if_present(
                            entry,
                            "Material",
                            NifValue::UInt(replacement),
                        ) {
                            changes.push(format!(
                                "{block_id} {field}[{index}]: Replaced material {material} with {replacement}"
                            ));
                        }
                    } else {
                        changes.push(format!("{block_id} {field}[{index}]: Material {material}"));
                    }
                }
                break;
            }
        } else if SCHEMA.is_subtype_of(&block.type_name, "bhkShape") {
            let Some(material) = value_u64(block.get_field("Material")) else {
                continue;
            };
            if search.is_some_and(|search| search != material) {
                continue;
            }
            if let Some(replacement) = replacement {
                if set_block_field_if_present(block, "Material", NifValue::UInt(replacement)) {
                    changes.push(format!(
                        "{block_id}: Replaced material {material} with {replacement}"
                    ));
                }
            } else {
                changes.push(format!("{block_id}: Material {material}"));
            }
        }
    }
    Ok(changes)
}

fn collision_target(nif: &NifFile, block_id: usize) -> Option<usize> {
    let mut current = block_id;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let owner = nif.blocks.iter().find(|block| {
            block
                .get_refs(&SCHEMA)
                .into_iter()
                .any(|reference| reference == current as i32)
        })?;
        if SCHEMA.is_subtype_of(&owner.type_name, "bhkCollisionObject") {
            return value_ref(owner.get_field("Target"))
                .filter(|target| *target >= 0)
                .map(|target| target as usize);
        }
        current = owner.block_id;
    }
    None
}

fn copy_controlled_blocks(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let Some(source) = load_source_nif(options, "source_file")? else {
        return Ok(Vec::new());
    };
    Ok(copy_controlled_blocks_from(nif, &source))
}

fn copy_controlled_blocks_from(nif: &mut NifFile, source: &NifFile) -> Vec<String> {
    let Some(source_root) = roots(&source).first().copied() else {
        return Vec::new();
    };
    let Some(destination_root) = roots(nif).first().copied() else {
        return Vec::new();
    };
    let source_entries = value_array(source.blocks[source_root].get_field("Controlled Blocks"));
    if source_entries.is_empty()
        || nif.blocks[destination_root]
            .get_field("Controlled Blocks")
            .is_none()
    {
        return Vec::new();
    }

    let mut destination_entries =
        value_array(nif.blocks[destination_root].get_field("Controlled Blocks")).to_vec();
    let mut destination_tokens = destination_entries
        .iter()
        .map(controlled_block_token)
        .collect::<HashSet<_>>();
    let mut copied_data = HashMap::<usize, usize>::new();
    let mut changes = Vec::new();

    for source_entry in source_entries {
        let token = controlled_block_token(source_entry);
        if !destination_tokens.insert(token) {
            continue;
        }
        let mut new_entry = source_entry.clone();
        if let Some(source_interpolator_id) = nested_value(Some(source_entry), "Interpolator")
            .and_then(|value| value_ref(Some(value)))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
            .filter(|reference| *reference < source.blocks.len())
        {
            let source_interpolator = &source.blocks[source_interpolator_id];
            let mut reference_map = HashMap::new();
            for reference in source_interpolator.get_refs(&SCHEMA) {
                if reference < 0 || reference as usize >= source.blocks.len() {
                    continue;
                }
                let source_reference_id = reference as usize;
                let destination_reference_id =
                    if let Some(existing) = copied_data.get(&source_reference_id) {
                        *existing
                    } else {
                        let copied = copy_block(&source, nif, source_reference_id);
                        copied_data.insert(source_reference_id, copied);
                        copied
                    };
                reference_map.insert(reference, destination_reference_id as i32);
            }
            let destination_interpolator_id = copy_block(&source, nif, source_interpolator_id);
            for value in nif.blocks[destination_interpolator_id].fields.values_mut() {
                remap_value_refs(value, &reference_map);
            }
            if let Some(interpolator) = nested_value_mut(Some(&mut new_entry), "Interpolator") {
                *interpolator = NifValue::Ref(destination_interpolator_id as i32);
            }
        }
        let name = nested_value(Some(&new_entry), "Node Name")
            .and_then(value_string)
            .unwrap_or_default();
        let controller = nested_value(Some(&new_entry), "Controller Type")
            .and_then(value_string)
            .unwrap_or_default();
        changes.push(format!(
            "{destination_root}: Copied controlled block {name:?} {controller:?}"
        ));
        destination_entries.push(new_entry);
    }
    if !changes.is_empty() {
        nif.blocks[destination_root].set_field(
            "Num Controlled Blocks",
            NifValue::UInt(destination_entries.len() as u64),
        );
        nif.blocks[destination_root]
            .set_field("Controlled Blocks", NifValue::Array(destination_entries));
    }
    changes
}

fn copy_priorities(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let Some(source) = load_source_nif(options, "source_file")? else {
        return Ok(Vec::new());
    };
    Ok(copy_priorities_from(nif, &source))
}

fn copy_priorities_from(nif: &mut NifFile, source: &NifFile) -> Vec<String> {
    let Some(source_root) = roots(&source).first().copied() else {
        return Vec::new();
    };
    let Some(destination_root) = roots(nif).first().copied() else {
        return Vec::new();
    };
    let priorities = value_array(source.blocks[source_root].get_field("Controlled Blocks"))
        .iter()
        .filter_map(|entry| {
            Some((
                nested_value(Some(entry), "Node Name")
                    .and_then(value_string)?
                    .to_ascii_lowercase(),
                nested_value(Some(entry), "Priority").and_then(|value| value_u64(Some(value)))?,
            ))
        })
        .collect::<Vec<_>>();
    let Some(NifValue::Array(entries)) =
        nif.blocks[destination_root].get_field_mut("Controlled Blocks")
    else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    for entry in entries {
        let Some(name) = nested_value(Some(entry), "Node Name")
            .and_then(value_string)
            .map(str::to_string)
        else {
            continue;
        };
        let Some((_, priority)) = priorities
            .iter()
            .find(|(source_name, _)| source_name.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        if nested_value(Some(entry), "Priority").and_then(|value| value_u64(Some(value)))
            == Some(*priority)
        {
            continue;
        }
        if let Some(value) = nested_value_mut(Some(entry), "Priority") {
            *value = NifValue::UInt(*priority);
            changes.push(format!(
                "{destination_root}: Set controlled block {name:?} priority to {priority}"
            ));
        }
    }
    changes
}

fn add_skeleton_blocks(
    nif: &mut NifFile,
    input: &Path,
    options: &Value,
) -> Result<Vec<String>, String> {
    if !input
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("death.kf"))
    {
        return Ok(Vec::new());
    }
    let skeleton_path = options
        .get("skeleton_file")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| input.with_file_name("skeleton.nif"));
    if !skeleton_path.is_file() {
        return Ok(Vec::new());
    }
    let skeleton = NifFile::load(&skeleton_path).map_err(|error| error.to_string())?;
    Ok(add_skeleton_blocks_from(nif, &skeleton, options))
}

fn add_skeleton_blocks_from(nif: &mut NifFile, skeleton: &NifFile, options: &Value) -> Vec<String> {
    let Some(root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    if nif.blocks[root_id].get_field("Controlled Blocks").is_none() {
        return Vec::new();
    }
    let mut names = options
        .get("names")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if names.is_empty() {
        names = vec!["weapon".to_string(), "headanims".to_string()];
    }
    let exact = option_bool(options, "exact_match").unwrap_or(true);
    let mut entries = value_array(nif.blocks[root_id].get_field("Controlled Blocks")).to_vec();
    let mut changes = Vec::new();

    for bone in skeleton
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiNode")
    {
        let Some(name) = bone
            .get_field("Name")
            .and_then(value_string)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if value_ref(bone.get_field("Collision Object")).is_some_and(|reference| reference >= 0) {
            continue;
        }
        let lower_name = name.to_ascii_lowercase();
        if names.iter().any(|candidate| {
            if exact {
                lower_name == *candidate
            } else {
                lower_name.contains(candidate)
            }
        }) {
            continue;
        }
        if entries.iter().any(|entry| {
            nested_value(Some(entry), "Node Name").and_then(value_string) == Some(name)
        }) {
            continue;
        }

        let interpolator_id = nif.add_block("NiTransformInterpolator", None);
        nif.blocks[interpolator_id].set_field("Transform", bone_quat_transform(bone));
        entries.push(NifValue::Struct(IndexMap::from([
            ("Node Name".to_string(), NifValue::String(name.to_string())),
            ("Priority".to_string(), NifValue::UInt(99)),
            (
                "Controller Type".to_string(),
                NifValue::String("NiTransformController".to_string()),
            ),
            (
                "Interpolator".to_string(),
                NifValue::Ref(interpolator_id as i32),
            ),
        ])));
        changes.push(format!("{root_id}: Added skeleton bone {name:?}"));
    }
    if !changes.is_empty() {
        nif.blocks[root_id].set_field(
            "Num Controlled Blocks",
            NifValue::UInt(entries.len() as u64),
        );
        nif.blocks[root_id].set_field("Controlled Blocks", NifValue::Array(entries));
    }
    changes
}

fn load_source_nif(options: &Value, option: &str) -> Result<Option<NifFile>, String> {
    let path = options
        .get(option)
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("{option} is required"))?;
    let path = Path::new(path);
    if !path.is_file() {
        return Ok(None);
    }
    NifFile::load(path)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn controlled_block_token(entry: &NifValue) -> String {
    let name = nested_value(Some(entry), "Node Name")
        .and_then(value_string)
        .unwrap_or_default();
    let controller = nested_value(Some(entry), "Controller Type")
        .and_then(value_string)
        .unwrap_or_default();
    format!("{name} {controller}").to_ascii_lowercase()
}

fn copy_block(source: &NifFile, destination: &mut NifFile, source_id: usize) -> usize {
    let source_block = &source.blocks[source_id];
    let destination_id = destination.add_block(
        source_block.type_name.clone(),
        Some(source_block.fields.clone()),
    );
    destination.blocks[destination_id].remainder = source_block.remainder.clone();
    destination_id
}

fn remap_value_refs(value: &mut NifValue, map: &HashMap<i32, i32>) {
    match value {
        NifValue::Ref(reference) => {
            if let Some(replacement) = map.get(reference) {
                *reference = *replacement;
            }
        }
        NifValue::Array(values) => {
            for value in values {
                remap_value_refs(value, map);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                remap_value_refs(value, map);
            }
        }
        _ => {}
    }
}

fn bone_quat_transform(bone: &NifBlock) -> NifValue {
    let translation = bone
        .get_field("Translation")
        .cloned()
        .unwrap_or(NifValue::Vec3([0.0, 0.0, 0.0]));
    let rotation = bone
        .get_field("Rotation")
        .and_then(matrix33_value)
        .map(matrix33_to_nif_quaternion)
        .map(NifValue::Quaternion)
        .unwrap_or(NifValue::Quaternion([1.0, 0.0, 0.0, 0.0]));
    let scale = bone
        .get_field("Scale")
        .cloned()
        .unwrap_or(NifValue::Float(1.0));
    NifValue::Struct(IndexMap::from([
        ("Translation".to_string(), translation),
        ("Rotation".to_string(), rotation),
        ("Scale".to_string(), scale),
    ]))
}

fn matrix33_value(value: &NifValue) -> Option<[[f32; 3]; 3]> {
    match value {
        NifValue::Matrix33(matrix) => Some(*matrix),
        _ => None,
    }
}

fn matrix33_to_nif_quaternion(matrix: [[f32; 3]; 3]) -> [f32; 4] {
    let trace = matrix[0][0] + matrix[1][1] + matrix[2][2];
    let [x, y, z, w] = if trace > 0.0 {
        let scale = (trace + 1.0).sqrt() * 2.0;
        [
            (matrix[2][1] - matrix[1][2]) / scale,
            (matrix[0][2] - matrix[2][0]) / scale,
            (matrix[1][0] - matrix[0][1]) / scale,
            0.25 * scale,
        ]
    } else if matrix[0][0] > matrix[1][1] && matrix[0][0] > matrix[2][2] {
        let scale = (1.0 + matrix[0][0] - matrix[1][1] - matrix[2][2]).sqrt() * 2.0;
        [
            0.25 * scale,
            (matrix[0][1] + matrix[1][0]) / scale,
            (matrix[0][2] + matrix[2][0]) / scale,
            (matrix[2][1] - matrix[1][2]) / scale,
        ]
    } else if matrix[1][1] > matrix[2][2] {
        let scale = (1.0 + matrix[1][1] - matrix[0][0] - matrix[2][2]).sqrt() * 2.0;
        [
            (matrix[0][1] + matrix[1][0]) / scale,
            0.25 * scale,
            (matrix[1][2] + matrix[2][1]) / scale,
            (matrix[0][2] - matrix[2][0]) / scale,
        ]
    } else {
        let scale = (1.0 + matrix[2][2] - matrix[0][0] - matrix[1][1]).sqrt() * 2.0;
        [
            (matrix[0][2] + matrix[2][0]) / scale,
            (matrix[1][2] + matrix[2][1]) / scale,
            0.25 * scale,
            (matrix[1][0] - matrix[0][1]) / scale,
        ]
    };
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if length > f32::EPSILON {
        [w / length, x / length, y / length, z / length]
    } else {
        [1.0, 0.0, 0.0, 0.0]
    }
}

fn remove_controlled_blocks(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let matcher = ControlledBlockMatcher::from_options(options)?;
    let sequence_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiSequence"))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    for sequence_id in sequence_ids {
        let Some(NifValue::Array(entries)) = nif.blocks[sequence_id]
            .get_field("Controlled Blocks")
            .cloned()
        else {
            continue;
        };
        let mut kept = Vec::with_capacity(entries.len());
        for entry in entries {
            let name = nested_value(Some(&entry), "Node Name")
                .and_then(value_string)
                .unwrap_or_default();
            if matcher.matches(name) {
                changes.push(format!("{sequence_id}: Removed controlled block {name:?}"));
            } else {
                kept.push(entry);
            }
        }
        if kept.len() != value_array(nif.blocks[sequence_id].get_field("Controlled Blocks")).len() {
            nif.blocks[sequence_id]
                .set_field("Num Controlled Blocks", NifValue::UInt(kept.len() as u64));
            nif.blocks[sequence_id].set_field("Controlled Blocks", NifValue::Array(kept));
        }
    }
    if !changes.is_empty()
        && roots(nif)
            .first()
            .and_then(|root| nif.get_block(*root))
            .is_some_and(|root| root.type_name == "NiControllerSequence")
    {
        changes.extend(remove_unused_nodes(nif, &json!({})));
    }
    Ok(changes)
}

fn quadratic_to_linear(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let matcher = ControlledBlockMatcher::from_options(options)?;
    let root_id = roots(nif).first().copied().unwrap_or(0);
    let entries = value_array(
        nif.get_block(root_id)
            .and_then(|root| root.get_field("Controlled Blocks")),
    )
    .to_vec();
    let mut data_ids = Vec::new();
    for entry in entries {
        let name = nested_value(Some(&entry), "Node Name")
            .and_then(value_string)
            .unwrap_or_default();
        if !matcher.matches(name) {
            continue;
        }
        let Some(interpolator_id) = nested_value(Some(&entry), "Interpolator")
            .and_then(|value| value_ref(Some(value)))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
        else {
            continue;
        };
        let Some(data_id) = nif
            .get_block(interpolator_id)
            .and_then(|block| value_ref(block.get_field("Data")))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
        else {
            continue;
        };
        data_ids.push((data_id, name.to_string()));
    }
    let mut changes = Vec::new();
    for (data_id, name) in data_ids {
        let Some(data) = nif.blocks.get_mut(data_id) else {
            continue;
        };
        if let Some(translations) = data.get_field_mut("Translations") {
            if replace_quadratic_interpolation(translations) {
                changes.push(format!("{data_id} {name:?}: Linearized translations"));
            }
        }
        if let Some(NifValue::Array(rotations)) = data.get_field_mut("XYZ Rotations") {
            for (index, rotation) in rotations.iter_mut().enumerate() {
                if replace_quadratic_interpolation(rotation) {
                    changes.push(format!(
                        "{data_id} {name:?}: Linearized XYZ rotation {index}"
                    ));
                }
            }
        }
    }
    Ok(changes)
}

fn replace_quadratic_interpolation(value: &mut NifValue) -> bool {
    let Some(interpolation) = nested_value_mut(Some(value), "Interpolation") else {
        return false;
    };
    if value_u64(Some(interpolation)) != Some(2) {
        return false;
    }
    *interpolation = NifValue::UInt(1);
    true
}

fn fix_exported_kf(nif: &mut NifFile) -> Vec<String> {
    let mut changes = Vec::new();
    for block in &mut nif.blocks {
        let block_id = block.block_id;
        if SCHEMA.is_subtype_of(&block.type_name, "NiSequence") {
            if let Some(NifValue::Array(entries)) = block.get_field_mut("Controlled Blocks") {
                for (index, entry) in entries.iter_mut().enumerate() {
                    if nested_value(Some(entry), "Controller Type")
                        .and_then(value_string)
                        .is_some_and(str::is_empty)
                    {
                        if let Some(value) = nested_value_mut(Some(entry), "Controller Type") {
                            *value = NifValue::String("NiTransformController".to_string());
                            changes.push(format!(
                                "{block_id} Controlled Blocks[{index}]: Set controller type"
                            ));
                        }
                    }
                }
            }
        } else if block.type_name == "NiTextKeyExtraData" {
            if let Some(NifValue::Array(entries)) = block.get_field_mut("Text Keys") {
                for (index, entry) in entries.iter_mut().enumerate() {
                    let should_fix = nested_value(Some(entry), "Value")
                        .and_then(value_string)
                        .is_some_and(|value| value.len() > 5 && value.starts_with("start"));
                    if should_fix {
                        if let Some(value) = nested_value_mut(Some(entry), "Value") {
                            *value = NifValue::String("start".to_string());
                            changes.push(format!(
                                "{block_id} Text Keys[{index}]: Truncated start key"
                            ));
                        }
                    }
                }
            }
        }
    }
    changes
}

fn optimize_animations(nif: &mut NifFile) -> Vec<String> {
    let data_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiKeyBasedInterpolator"))
        .filter_map(|block| value_ref(block.get_field("Data")))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .collect::<HashSet<_>>();
    let mut changes = Vec::new();
    for data_id in data_ids {
        let Some(data) = nif.blocks.get_mut(data_id) else {
            continue;
        };
        if optimize_key_array(data.get_field_mut("Quaternion Keys")) {
            let count = value_array(data.get_field("Quaternion Keys")).len();
            data.set_field("Num Rotation Keys", NifValue::UInt(count as u64));
            changes.push(format!("{data_id}: Optimized quaternion keys"));
        }
        for group in ["Translations", "Scales"] {
            if let Some(value) = data.get_field_mut(group) {
                if optimize_nested_keys(value) {
                    changes.push(format!("{data_id}: Optimized {group}"));
                }
            }
        }
        if let Some(NifValue::Array(rotations)) = data.get_field_mut("XYZ Rotations") {
            for (index, rotation) in rotations.iter_mut().enumerate() {
                if optimize_nested_keys(rotation) {
                    changes.push(format!("{data_id}: Optimized XYZ rotation {index}"));
                }
            }
        }
    }
    changes
}

fn optimize_nested_keys(value: &mut NifValue) -> bool {
    let Some(keys) = nested_value_mut(Some(value), "Keys") else {
        return false;
    };
    if !optimize_key_array(Some(keys)) {
        return false;
    }
    let count = value_array(nested_value(Some(value), "Keys")).len();
    if let Some(num_keys) = nested_value_mut(Some(value), "Num Keys") {
        *num_keys = NifValue::UInt(count as u64);
    }
    true
}

fn optimize_key_array(value: Option<&mut NifValue>) -> bool {
    let Some(NifValue::Array(keys)) = value else {
        return false;
    };
    if keys.len() < 3 {
        return false;
    }
    let mut changed = false;
    for index in (1..keys.len() - 1).rev() {
        if key_value(&keys[index - 1]) == key_value(&keys[index])
            && key_value(&keys[index]) == key_value(&keys[index + 1])
        {
            keys.remove(index);
            changed = true;
        }
    }
    changed
}

fn add_transform_data(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let add_rotation = option_bool(options, "add_rotation").unwrap_or(true);
    let add_translation = option_bool(options, "add_translation").unwrap_or(true);
    if !add_rotation && !add_translation {
        return Err("Need to select at least one option".to_string());
    }
    let stop_time = roots(nif)
        .first()
        .and_then(|root_id| nif.get_block(*root_id))
        .and_then(|root| value_f64(root.get_field("Stop Time")))
        .unwrap_or(0.0);
    let interpolators = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiTransformInterpolator")
        .filter(|block| value_ref(block.get_field("Data")).is_none_or(|reference| reference < 0))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    for interpolator_id in interpolators {
        let transform = nif.blocks[interpolator_id].get_field("Transform").cloned();
        let rotation = transform
            .as_ref()
            .and_then(|value| nested_value(Some(value), "Rotation"))
            .cloned();
        let translation = transform
            .as_ref()
            .and_then(|value| nested_value(Some(value), "Translation"))
            .filter(|value| vec3_value(value).is_some_and(|v| v.iter().all(|x| x.is_finite())))
            .cloned();
        let data_id = nif.add_block("NiTransformData", None);
        nif.blocks[interpolator_id].set_field("Data", NifValue::Ref(data_id as i32));
        if add_rotation {
            if let Some(rotation) = rotation {
                nif.blocks[data_id].set_field("Num Rotation Keys", NifValue::UInt(2));
                nif.blocks[data_id].set_field("Rotation Type", NifValue::UInt(1));
                nif.blocks[data_id].set_field(
                    "Quaternion Keys",
                    NifValue::Array(vec![
                        animation_key(0.0, rotation.clone()),
                        animation_key(stop_time, rotation),
                    ]),
                );
            }
        }
        if add_translation {
            if let Some(translation) = translation {
                nif.blocks[data_id].set_field(
                    "Translations",
                    NifValue::Struct(IndexMap::from([
                        ("Num Keys".to_string(), NifValue::UInt(2)),
                        ("Interpolation".to_string(), NifValue::UInt(1)),
                        (
                            "Keys".to_string(),
                            NifValue::Array(vec![
                                animation_key(0.0, translation.clone()),
                                animation_key(stop_time, translation),
                            ]),
                        ),
                    ])),
                );
            }
        }
        changes.push(format!(
            "{interpolator_id}: Added NiTransformData {data_id}"
        ));
    }
    Ok(changes)
}

fn animation_key(time: f64, value: NifValue) -> NifValue {
    NifValue::Struct(IndexMap::from([
        ("Time".to_string(), NifValue::Float(time)),
        ("Value".to_string(), value),
    ]))
}

fn add_headtracking_anim(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let Some(root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    if nif.blocks[root_id].type_name != "NiControllerSequence" {
        return Vec::new();
    }
    if option_bool(options, "cycle_clamp_only").unwrap_or(false)
        && value_u64(nif.blocks[root_id].get_field("Cycle Type")) != Some(2)
    {
        return Vec::new();
    }
    let entries = value_array(nif.blocks[root_id].get_field("Controlled Blocks"));
    let mut priority = None;
    for entry in entries {
        if nested_value(Some(entry), "Node Name").and_then(value_string) != Some("Bip01 Head") {
            continue;
        }
        if nested_value(Some(entry), "Controller Type").and_then(value_string)
            == Some("NiFloatExtraDataController")
        {
            return Vec::new();
        }
        priority = nested_value(Some(entry), "Priority").and_then(|value| value_u64(Some(value)));
    }
    let Some(priority) = priority else {
        return Vec::new();
    };
    let stop_time = value_f64(nif.blocks[root_id].get_field("Stop Time")).unwrap_or(0.0);
    let key_value_14 = option_f64(options, "key_value_14").unwrap_or(0.0);
    let key_value_23 = option_f64(options, "key_value_23").unwrap_or(100.0);
    let key_time_2 = option_f64(options, "key_time_2").unwrap_or(20.0);
    let key_time_3 = option_f64(options, "key_time_3").unwrap_or(80.0);
    let round_time = |percent: f64| {
        let frame = 1.0 / 30.0;
        (stop_time * (percent / 100.0) / frame).round() * frame
    };
    let interpolator_id = nif.add_block("NiFloatInterpolator", None);
    let data_id = nif.add_block("NiFloatData", None);
    nif.blocks[interpolator_id].set_field("Data", NifValue::Ref(data_id as i32));
    nif.blocks[data_id].set_field(
        "Data",
        NifValue::Struct(IndexMap::from([
            ("Num Keys".to_string(), NifValue::UInt(4)),
            ("Interpolation".to_string(), NifValue::UInt(1)),
            (
                "Keys".to_string(),
                NifValue::Array(vec![
                    animation_key(0.0, NifValue::Float(key_value_14)),
                    animation_key(round_time(key_time_2), NifValue::Float(key_value_23)),
                    animation_key(round_time(key_time_3), NifValue::Float(key_value_23)),
                    animation_key(stop_time, NifValue::Float(key_value_14)),
                ]),
            ),
        ])),
    );
    let mut controlled = value_array(nif.blocks[root_id].get_field("Controlled Blocks")).to_vec();
    controlled.push(NifValue::Struct(IndexMap::from([
        (
            "Node Name".to_string(),
            NifValue::String("Bip01 Head".to_string()),
        ),
        ("Priority".to_string(), NifValue::UInt(priority)),
        (
            "Controller Type".to_string(),
            NifValue::String("NiFloatExtraDataController".to_string()),
        ),
        (
            "Controller ID".to_string(),
            NifValue::String("HeadTrack".to_string()),
        ),
        (
            "Interpolator".to_string(),
            NifValue::Ref(interpolator_id as i32),
        ),
    ])));
    nif.blocks[root_id].set_field(
        "Num Controlled Blocks",
        NifValue::UInt(controlled.len() as u64),
    );
    nif.blocks[root_id].set_field("Controlled Blocks", NifValue::Array(controlled));
    vec![format!(
        "{root_id}: Added Bip01 Head HeadTrack controller using {interpolator_id}/{data_id}"
    )]
}

struct FacialModifier {
    priority: u64,
    modifier: String,
    keys: Vec<(f64, f64)>,
}

fn add_facial_anim(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    const DEFAULT_MODIFIERS: &[&str] = &[
        "99 Aah 0.466667 0 1.499999 1",
        "99 Eh 1.500000 1",
        "99 BigAah 1.5 1 1.833333 0.1 3.333333 0.5 4.033333 1",
    ];
    const MODIFIERS: &[&str] = &[
        "Anger",
        "Fear",
        "Happy",
        "Sad",
        "Surprise",
        "MoodNeutral",
        "MoodAfraid",
        "MoodAnnoyed",
        "MoodCocky",
        "MoodDrugged",
        "MoodPleasant",
        "MoodAngry",
        "MoodSad",
        "Pained",
        "CombatAnger",
        "Aah",
        "BigAah",
        "BMP",
        "ChjSh",
        "DST",
        "Eee",
        "Eh",
        "FV",
        "i",
        "k",
        "N",
        "Oh",
        "OohQ",
        "R",
        "Th",
        "W",
        "BlinkLeft",
        "BlinkRight",
        "BrowDownLeft",
        "BrowDownRight",
        "BrowInLeft",
        "BrowInRight",
        "BrowUpLeft",
        "BrowUpRight",
        "LookDown",
        "LookLeft",
        "LookRight",
        "LookUp",
        "SquintLeft",
        "SquintRight",
        "HeadPitch",
        "HeadRoll",
        "HeadYaw",
    ];
    let modifier_lines = if let Some(values) = options.get("facial_mods").and_then(Value::as_array)
    {
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("Facial modifier {} is not text", index + 1))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        DEFAULT_MODIFIERS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    };
    let modifiers = modifier_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let values = line.split_whitespace().collect::<Vec<_>>();
            if values.len() < 4 {
                return Err(format!(
                    "Facial modifier line {} has fewer than 4 values",
                    index + 1
                ));
            }
            if (values.len() - 2) % 2 != 0 {
                return Err(format!(
                    "Facial modifier line {} has an unmatched time/intensity value",
                    index + 1
                ));
            }
            let priority = values[0]
                .parse::<u64>()
                .map_err(|_| format!("Invalid priority {:?} on line {}", values[0], index + 1))?;
            let modifier = values[1];
            if !MODIFIERS.contains(&modifier) {
                return Err(format!(
                    "Invalid expression/phoneme/modifier {modifier:?} on line {}",
                    index + 1
                ));
            }
            let keys = values[2..]
                .chunks_exact(2)
                .map(|pair| {
                    Ok((
                        pair[0].parse::<f64>().map_err(|_| {
                            format!("Invalid time {:?} on line {}", pair[0], index + 1)
                        })?,
                        pair[1].parse::<f64>().map_err(|_| {
                            format!("Invalid intensity {:?} on line {}", pair[1], index + 1)
                        })?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(FacialModifier {
                priority,
                modifier: modifier.to_string(),
                keys,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let Some(root_id) = roots(nif).first().copied() else {
        return Ok(Vec::new());
    };
    if nif.blocks[root_id].type_name != "NiControllerSequence" {
        return Ok(Vec::new());
    }
    let remove_existing = option_bool(options, "remove_existing_facial").unwrap_or(false);
    let mut changes = Vec::new();
    if remove_existing {
        let reachable_before = reachable_blocks(nif);
        let mut controlled =
            value_array(nif.blocks[root_id].get_field("Controlled Blocks")).to_vec();
        let old_len = controlled.len();
        controlled.retain(|entry| {
            nested_value(Some(entry), "Node Name").and_then(value_string) != Some("HeadAnims:0")
        });
        if controlled.len() != old_len {
            nif.blocks[root_id].set_field(
                "Num Controlled Blocks",
                NifValue::UInt(controlled.len() as u64),
            );
            nif.blocks[root_id].set_field("Controlled Blocks", NifValue::Array(controlled));
            let reachable_after = reachable_blocks(nif);
            let removed = reachable_before
                .difference(&reachable_after)
                .copied()
                .collect::<Vec<_>>();
            nif.remove_blocks(&removed);
            changes.push(format!("{root_id}: Removed existing facial animations"));
        }
    }
    let root_id = roots(nif).first().copied().unwrap_or(root_id);
    let has_head_anim = value_array(nif.blocks[root_id].get_field("Controlled Blocks"))
        .iter()
        .any(|entry| {
            nested_value(Some(entry), "Node Name").and_then(value_string) == Some("HeadAnims")
        });
    let mut controlled = value_array(nif.blocks[root_id].get_field("Controlled Blocks")).to_vec();
    if !has_head_anim {
        let interpolator_id = nif.add_block("NiBoolInterpolator", None);
        nif.blocks[interpolator_id].set_field("Value", NifValue::Bool(true));
        controlled.push(NifValue::Struct(IndexMap::from([
            (
                "Node Name".to_string(),
                NifValue::String("HeadAnims".to_string()),
            ),
            (
                "Controller Type".to_string(),
                NifValue::String("NiVisController".to_string()),
            ),
            (
                "Interpolator".to_string(),
                NifValue::Ref(interpolator_id as i32),
            ),
            (
                "Priority".to_string(),
                NifValue::UInt(modifiers.first().map_or(0, |modifier| modifier.priority)),
            ),
        ])));
        changes.push(format!("{root_id}: Added HeadAnims visibility controller"));
    }
    for modifier in modifiers {
        let interpolator_id = nif.add_block("NiFloatInterpolator", None);
        nif.blocks[interpolator_id].set_field("Value", NifValue::Float(0.0));
        let data_id = nif.add_block("NiFloatData", None);
        nif.blocks[interpolator_id].set_field("Data", NifValue::Ref(data_id as i32));
        let keys = modifier
            .keys
            .into_iter()
            .map(|(time, value)| {
                NifValue::Struct(IndexMap::from([
                    ("Time".to_string(), NifValue::Float(time)),
                    ("Value".to_string(), NifValue::Float(value)),
                    ("Forward".to_string(), NifValue::Float(0.0)),
                    ("Backward".to_string(), NifValue::Float(0.0)),
                ]))
            })
            .collect::<Vec<_>>();
        nif.blocks[data_id].set_field(
            "Data",
            NifValue::Struct(IndexMap::from([
                ("Num Keys".to_string(), NifValue::UInt(keys.len() as u64)),
                ("Interpolation".to_string(), NifValue::UInt(2)),
                ("Keys".to_string(), NifValue::Array(keys)),
            ])),
        );
        controlled.push(NifValue::Struct(IndexMap::from([
            (
                "Node Name".to_string(),
                NifValue::String("HeadAnims:0".to_string()),
            ),
            ("Priority".to_string(), NifValue::UInt(modifier.priority)),
            (
                "Controller Type".to_string(),
                NifValue::String("NiGeomMorpherController".to_string()),
            ),
            (
                "Interpolator ID".to_string(),
                NifValue::String(modifier.modifier.clone()),
            ),
            (
                "Interpolator".to_string(),
                NifValue::Ref(interpolator_id as i32),
            ),
        ])));
        changes.push(format!(
            "{root_id}: Added facial modifier {:?}",
            modifier.modifier
        ));
    }
    nif.blocks[root_id].set_field(
        "Num Controlled Blocks",
        NifValue::UInt(controlled.len() as u64),
    );
    nif.blocks[root_id].set_field("Controlled Blocks", NifValue::Array(controlled));
    Ok(changes)
}

fn weijiesen_blow_up(nif: &mut NifFile) -> Vec<String> {
    let Some(root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    let Some(non_accum_id) = nif.blocks.iter().find_map(|block| {
        (block.type_name == "NiNode"
            && block
                .get_field("Name")
                .and_then(value_string)
                .is_some_and(|name| name.ends_with("NonAccum")))
        .then_some(block.block_id)
    }) else {
        return Vec::new();
    };
    let Some(controller_id) = first_block_id(nif, "NiMultiTargetTransformController") else {
        return Vec::new();
    };
    let Some(palette_id) = first_block_id(nif, "NiDefaultAVObjectPalette") else {
        return Vec::new();
    };
    let Some(sequence_id) = first_block_id(nif, "NiControllerSequence") else {
        return Vec::new();
    };

    let root_children = value_array(nif.blocks[root_id].get_field("Children")).to_vec();
    let moved_children = root_children
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|child_id| {
            *child_id != non_accum_id
                && nif
                    .blocks
                    .get(*child_id)
                    .is_some_and(|block| block.type_name == "NiNode")
        })
        .collect::<Vec<_>>();
    if moved_children.is_empty() {
        return Vec::new();
    }
    let moved_set = moved_children.iter().copied().collect::<HashSet<_>>();
    let remaining_root_children = root_children
        .into_iter()
        .filter(|value| {
            value_ref(Some(value))
                .filter(|reference| *reference >= 0)
                .is_none_or(|reference| !moved_set.contains(&(reference as usize)))
        })
        .collect::<Vec<_>>();
    let moved_refs = moved_children
        .iter()
        .map(|child_id| NifValue::Ref(*child_id as i32))
        .collect::<Vec<_>>();

    nif.blocks[non_accum_id].set_field("Num Children", NifValue::UInt(moved_children.len() as u64));
    nif.blocks[non_accum_id].set_field("Children", NifValue::Array(moved_refs.clone()));
    nif.blocks[root_id].set_field(
        "Num Children",
        NifValue::UInt(remaining_root_children.len() as u64),
    );
    nif.blocks[root_id].set_field("Children", NifValue::Array(remaining_root_children));
    nif.blocks[controller_id].set_field(
        "Num Extra Targets",
        NifValue::UInt(moved_children.len() as u64),
    );
    nif.blocks[controller_id].set_field("Extra Targets", NifValue::Array(moved_refs));

    let palette_objects = moved_children
        .iter()
        .map(|child_id| {
            let name = nif.blocks[*child_id]
                .get_field("Name")
                .and_then(value_string)
                .unwrap_or_default();
            NifValue::Struct(IndexMap::from([
                ("Name".to_string(), NifValue::String(name.to_string())),
                ("AV Object".to_string(), NifValue::Ref(*child_id as i32)),
            ]))
        })
        .collect::<Vec<_>>();
    nif.blocks[palette_id].set_field("Num Objs", NifValue::UInt(palette_objects.len() as u64));
    nif.blocks[palette_id].set_field("Objs", NifValue::Array(palette_objects));

    let mut controllers_to_remove = Vec::new();
    let controlled_blocks = moved_children
        .iter()
        .map(|child_id| {
            let child = &nif.blocks[*child_id];
            let name = child
                .get_field("Name")
                .and_then(value_string)
                .unwrap_or_default();
            let child_controller_id = value_ref(child.get_field("Controller"))
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize);
            let interpolator = child_controller_id
                .and_then(|id| nif.blocks.get(id))
                .and_then(|block| value_ref(block.get_field("Interpolator")))
                .unwrap_or(-1);
            if let Some(id) = child_controller_id {
                controllers_to_remove.push(id);
            }
            NifValue::Struct(IndexMap::from([
                ("Node Name".to_string(), NifValue::String(name.to_string())),
                (
                    "Controller".to_string(),
                    NifValue::Ref(controller_id as i32),
                ),
                ("Interpolator".to_string(), NifValue::Ref(interpolator)),
            ]))
        })
        .collect::<Vec<_>>();
    nif.blocks[sequence_id].set_field(
        "Num Controlled Blocks",
        NifValue::UInt(controlled_blocks.len() as u64),
    );
    nif.blocks[sequence_id].set_field("Controlled Blocks", NifValue::Array(controlled_blocks));

    controllers_to_remove.sort_unstable();
    controllers_to_remove.dedup();
    nif.remove_blocks(&controllers_to_remove);
    vec![format!(
        "Moved {} root NiNode(s) under NonAccum and rebuilt explosion controllers",
        moved_children.len()
    )]
}

fn first_block_id(nif: &NifFile, block_type: &str) -> Option<usize> {
    nif.blocks
        .iter()
        .find(|block| block.type_name == block_type)
        .map(|block| block.block_id)
}

fn key_value(key: &NifValue) -> Vec<Option<NifValue>> {
    ["Value", "Forward", "Backward", "TBC"]
        .iter()
        .map(|field| nested_value(Some(key), field).cloned())
        .collect()
}

struct ControlledBlockMatcher {
    names: Vec<String>,
    exact: bool,
    not_matching: bool,
}

impl ControlledBlockMatcher {
    fn from_options(options: &Value) -> Result<Self, String> {
        let names = options
            .get("names")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if names.is_empty() {
            return Err("Names field can not be empty".to_string());
        }
        Ok(Self {
            names,
            exact: option_bool(options, "exact_match").unwrap_or(true),
            not_matching: option_bool(options, "not_matching").unwrap_or(false),
        })
    }

    fn matches(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        let matched = self.names.iter().any(|candidate| {
            if self.exact {
                name == *candidate
            } else {
                name.contains(candidate)
            }
        });
        matched ^ self.not_matching
    }
}

fn remove_nodes(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let block_type = options
        .get("node_type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let matcher = if block_type.is_none() {
        Some(ControlledBlockMatcher::from_options(options)?)
    } else {
        None
    };
    if let Some(block_type) = block_type {
        if !SCHEMA.is_known_type(block_type) {
            return Err(format!("Unknown NIF block type: {block_type}"));
        }
    }
    let targets = nif
        .blocks
        .iter()
        .filter(|block| {
            if let Some(block_type) = block_type {
                SCHEMA.is_subtype_of(&block.type_name, block_type)
            } else {
                let name = block
                    .get_field("Name")
                    .and_then(value_string)
                    .unwrap_or_default();
                matcher.as_ref().unwrap().matches(name)
            }
        })
        .map(|block| block.block_id)
        .collect::<HashSet<_>>();
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let reachable_before = reachable_blocks(nif);
    for block in &mut nif.blocks {
        for value in block.fields.values_mut() {
            unlink_refs(value, &targets);
        }
    }
    let reachable = reachable_blocks(nif);
    let removed = nif
        .blocks
        .iter()
        .filter(|block| {
            targets.contains(&block.block_id)
                || (reachable_before.contains(&block.block_id)
                    && !reachable.contains(&block.block_id))
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let removed_names = removed
        .iter()
        .filter_map(|block_id| nif.get_block(*block_id))
        .map(|block| format!("{} {}", block.block_id, block.type_name))
        .collect::<Vec<_>>();
    nif.remove_blocks(&removed);
    for block in &mut nif.blocks {
        for value in block.fields.values_mut() {
            collapse_null_ref_arrays(value);
        }
    }
    Ok(removed_names
        .into_iter()
        .map(|block| format!("Removed {block}"))
        .collect())
}

fn unlink_refs(value: &mut NifValue, targets: &HashSet<usize>) {
    match value {
        NifValue::Ref(reference) if *reference >= 0 && targets.contains(&(*reference as usize)) => {
            *reference = -1;
        }
        NifValue::Array(values) => {
            for value in values {
                unlink_refs(value, targets);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                unlink_refs(value, targets);
            }
        }
        _ => {}
    }
}

fn reachable_blocks(nif: &NifFile) -> HashSet<usize> {
    let mut reachable = HashSet::new();
    let mut pending = roots(nif);
    while let Some(block_id) = pending.pop() {
        if !reachable.insert(block_id) {
            continue;
        }
        if let Some(block) = nif.get_block(block_id) {
            pending.extend(
                block
                    .get_refs(&SCHEMA)
                    .into_iter()
                    .filter(|reference| *reference >= 0)
                    .map(|reference| reference as usize),
            );
        }
    }
    reachable
}

fn collapse_null_ref_arrays(value: &mut NifValue) {
    match value {
        NifValue::Array(values) => {
            if values.iter().any(|value| matches!(value, NifValue::Ref(_))) {
                values.retain(|value| !matches!(value, NifValue::Ref(reference) if *reference < 0));
            }
            for value in values {
                collapse_null_ref_arrays(value);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                collapse_null_ref_arrays(value);
            }
        }
        _ => {}
    }
}

fn attach_parent(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let find_name = options
        .get("find_name")
        .and_then(Value::as_str)
        .unwrap_or("##SightingNode");
    let parent_name = options
        .get("parent_name")
        .and_then(Value::as_str)
        .unwrap_or("##ISControl");
    if find_name.is_empty() {
        return Err("Name to find can not be empty".to_string());
    }
    if nif.blocks.iter().any(|block| {
        block
            .get_field("Name")
            .and_then(value_string)
            .is_some_and(|name| name == parent_name)
    }) {
        return Ok(Vec::new());
    }
    let Some(child_id) = nif.blocks.iter().find_map(|block| {
        block
            .get_field("Name")
            .and_then(value_string)
            .is_some_and(|name| name == find_name)
            .then_some(block.block_id)
    }) else {
        return Ok(Vec::new());
    };
    let Some(owner_id) = nif.blocks.iter().find_map(|block| {
        SCHEMA
            .is_subtype_of(&block.type_name, "NiNode")
            .then(|| {
                value_array(block.get_field("Children"))
                    .iter()
                    .any(|value| value_ref(Some(value)) == Some(child_id as i32))
                    .then_some(block.block_id)
            })
            .flatten()
    }) else {
        return Ok(Vec::new());
    };
    let parent_id = nif.insert_block(child_id, "NiNode");
    let shifted_child_id = child_id + 1;
    let shifted_owner_id = owner_id + usize::from(owner_id >= child_id);
    nif.blocks[parent_id].set_field("Name", NifValue::String(parent_name.to_string()));
    nif.blocks[parent_id].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[parent_id].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shifted_child_id as i32)]),
    );
    if let Some(NifValue::Array(children)) = nif.blocks[shifted_owner_id].get_field_mut("Children")
    {
        for child in children {
            if value_ref(Some(child)) == Some(shifted_child_id as i32) {
                *child = NifValue::Ref(parent_id as i32);
                break;
            }
        }
    }
    Ok(vec![format!(
        "{parent_id}: Attached parent {parent_name:?} above {find_name:?}"
    )])
}

fn add_bounding_box(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let Some(root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    if nif.blocks[root_id].get_field("Children").is_none() {
        return Vec::new();
    }
    if value_array(nif.blocks[root_id].get_field("Children"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .filter_map(|reference| nif.get_block(reference as usize))
        .any(|child| {
            child
                .get_field("Name")
                .and_then(value_string)
                .is_some_and(|name| name == "Bounding Box")
        })
    {
        return Vec::new();
    }
    let flags = option_u64(options, "bounding_flags").unwrap_or(12);
    let center = option_vec3(options, "center").unwrap_or([0.0; 3]);
    let extent = option_vec3(options, "extent").unwrap_or([0.0; 3]);
    let child_id = nif.add_block(
        "NiNode",
        Some(IndexMap::from([
            (
                "Name".to_string(),
                NifValue::String("Bounding Box".to_string()),
            ),
            ("Flags".to_string(), NifValue::UInt(flags)),
            ("Has Bounding Volume".to_string(), NifValue::Bool(true)),
            (
                "Bounding Volume".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("Collision Type".to_string(), NifValue::UInt(1)),
                    (
                        "Box".to_string(),
                        NifValue::Struct(IndexMap::from([
                            ("Center".to_string(), NifValue::Vec3(center)),
                            (
                                "Axis".to_string(),
                                NifValue::Array(vec![
                                    NifValue::Vec3([1.0, 0.0, 0.0]),
                                    NifValue::Vec3([0.0, 1.0, 0.0]),
                                    NifValue::Vec3([0.0, 0.0, 1.0]),
                                ]),
                            ),
                            ("Extent".to_string(), NifValue::Vec3(extent)),
                        ])),
                    ),
                ])),
            ),
        ])),
    );
    append_ref(&mut nif.blocks[root_id], "Children", child_id);
    vec![format!("{child_id}: Added Bounding Box node")]
}

fn adjust_transform(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let mode = options
        .get("transform_mode")
        .and_then(Value::as_str)
        .unwrap_or("add");
    if !matches!(mode, "add" | "multiply" | "set") {
        return Err(format!("Invalid transform mode: {mode}"));
    }
    let adjustments = [
        option_f64(options, "translate_x"),
        option_f64(options, "translate_y"),
        option_f64(options, "translate_z"),
        option_f64(options, "yaw"),
        option_f64(options, "pitch"),
        option_f64(options, "roll"),
        option_f64(options, "scale"),
    ];
    if adjustments.iter().all(Option::is_none) {
        return Err("No adjustment values set".to_string());
    }
    let names = options
        .get("names")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let exact = option_bool(options, "exact_match").unwrap_or(true);
    let block_ids: Vec<usize> = if names.is_empty() {
        roots(nif).first().copied().into_iter().collect()
    } else {
        nif.blocks
            .iter()
            .filter(|block| {
                let name = block
                    .get_field("Name")
                    .and_then(value_string)
                    .unwrap_or_default()
                    .to_lowercase();
                names.iter().any(|candidate| {
                    if exact {
                        name == *candidate
                    } else {
                        name.contains(candidate)
                    }
                })
            })
            .map(|block| block.block_id)
            .collect()
    };
    let mut changes = Vec::new();
    for block_id in block_ids {
        let block = &mut nif.blocks[block_id];
        let mut changed = false;
        if let Some(NifValue::Vec3(translation)) = block.get_field_mut("Translation") {
            for (value, adjustment) in translation.iter_mut().zip(&adjustments[..3]) {
                if let Some(adjustment) = adjustment {
                    let updated = adjust_number(*value as f64, *adjustment, mode) as f32;
                    changed |= updated != *value;
                    *value = updated;
                }
            }
        }
        if let Some(NifValue::Float(value)) = block.get_field_mut("Scale") {
            if let Some(adjustment) = adjustments[6] {
                let updated = adjust_number(*value, adjustment, mode);
                changed |= updated != *value;
                *value = updated;
            }
        }
        if adjustments[3..6].iter().any(Option::is_some) {
            if let Some(NifValue::Matrix33(rotation)) = block.get_field_mut("Rotation") {
                let mut euler = matrix_to_euler_degrees(*rotation);
                for (value, adjustment) in euler.iter_mut().zip(&adjustments[3..6]) {
                    if let Some(adjustment) = adjustment {
                        *value = adjust_number(*value, *adjustment, mode);
                    }
                }
                let updated = euler_degrees_to_matrix(euler);
                changed |= updated != *rotation;
                *rotation = updated;
            }
        }
        if changed {
            changes.push(format!("{block_id}: Adjusted transformation"));
        }
    }
    Ok(changes)
}

fn adjust_number(current: f64, adjustment: f64, mode: &str) -> f64 {
    match mode {
        "multiply" => current * adjustment,
        "set" => adjustment,
        _ => current + adjustment,
    }
}

fn copy_geometry_blocks(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let copy_geometry = option_bool(options, "copy_geometry").unwrap_or(true);
    let copy_transform = option_bool(options, "copy_transform").unwrap_or(false);
    let copy_shader = option_bool(options, "copy_shader").unwrap_or(false);
    let copy_texture_set = option_bool(options, "copy_texture_set").unwrap_or(false);
    if !copy_geometry && !copy_transform && !copy_shader {
        return Err("Nothing to copy".to_string());
    }
    let Some(source) = load_source_nif(options, "source_file")? else {
        return Ok(Vec::new());
    };
    Ok(copy_geometry_blocks_from(
        nif,
        &source,
        copy_geometry,
        copy_transform,
        copy_shader,
        copy_texture_set,
    ))
}

fn copy_geometry_blocks_from(
    nif: &mut NifFile,
    source: &NifFile,
    copy_geometry: bool,
    copy_transform: bool,
    copy_shader: bool,
    copy_texture_set: bool,
) -> Vec<String> {
    let destination_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiAVObject"))
        .filter(|block| {
            block
                .get_field("Name")
                .and_then(value_string)
                .is_some_and(|name| !name.is_empty())
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut removed_tangent_blocks = Vec::new();

    for destination_id in destination_ids {
        let name = nif.blocks[destination_id]
            .get_field("Name")
            .and_then(value_string)
            .unwrap_or_default()
            .to_string();
        let Some(source_id) = source.blocks.iter().find_map(|block| {
            (block.get_field("Name").and_then(value_string) == Some(name.as_str()))
                .then_some(block.block_id)
        }) else {
            continue;
        };
        if nif.blocks[destination_id].type_name != source.blocks[source_id].type_name {
            continue;
        }
        let mut copied = Vec::new();

        if copy_transform {
            for field in ["Translation", "Rotation", "Scale"] {
                if let Some(value) = source.blocks[source_id].get_field(field).cloned() {
                    nif.blocks[destination_id].set_field(field, value);
                }
            }
            copied.push("transform");
        }
        if copy_shader
            && (SCHEMA.is_subtype_of(&nif.blocks[destination_id].type_name, "NiGeometry")
                || SCHEMA.is_subtype_of(&nif.blocks[destination_id].type_name, "BSTriShape"))
        {
            if copy_shader_fields(nif, source, destination_id, source_id, copy_texture_set) {
                copied.push(if copy_texture_set {
                    "shader and texture set"
                } else {
                    "shader"
                });
            }
        }
        if copy_geometry {
            if SCHEMA.is_subtype_of(&nif.blocks[destination_id].type_name, "NiTriBasedGeom") {
                if copy_legacy_geometry(
                    nif,
                    source,
                    destination_id,
                    source_id,
                    &mut removed_tangent_blocks,
                ) {
                    copied.push("geometry");
                }
            } else if SCHEMA.is_subtype_of(&nif.blocks[destination_id].type_name, "BSTriShape") {
                copy_modern_geometry(nif, source, destination_id, source_id);
                copied.push("geometry");
            }
        }
        if !copied.is_empty() {
            changes.push(format!(
                "{destination_id} {name:?}: Copied {}",
                copied.join(", ")
            ));
        }
    }
    if !removed_tangent_blocks.is_empty() {
        removed_tangent_blocks.sort_unstable();
        removed_tangent_blocks.dedup();
        nif.remove_blocks(&removed_tangent_blocks);
    }
    changes
}

fn copy_shader_fields(
    nif: &mut NifFile,
    source: &NifFile,
    destination_shape_id: usize,
    source_shape_id: usize,
    copy_texture_set: bool,
) -> bool {
    let Some(source_shader_id) = geometry_shader_id(source, source_shape_id) else {
        return false;
    };
    let Some(destination_shader_id) = geometry_shader_id(nif, destination_shape_id) else {
        return false;
    };
    if source.blocks[source_shader_id].type_name != nif.blocks[destination_shader_id].type_name {
        return false;
    }
    let source_fields = source.blocks[source_shader_id].fields.clone();
    for (field, value) in source_fields {
        let bare = bare_name(&field);
        if bare.starts_with("Extra Data") || matches!(bare, "Name" | "Controller" | "Texture Set") {
            continue;
        }
        if nif.blocks[destination_shader_id].get_field(bare).is_some() {
            nif.blocks[destination_shader_id].set_field(bare, value);
        }
    }
    if copy_texture_set {
        let source_set_id = value_ref(source.blocks[source_shader_id].get_field("Texture Set"))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize);
        let destination_set_id =
            value_ref(nif.blocks[destination_shader_id].get_field("Texture Set"))
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize);
        if let (Some(source_set_id), Some(destination_set_id)) = (source_set_id, destination_set_id)
        {
            if source_set_id < source.blocks.len() && destination_set_id < nif.blocks.len() {
                nif.blocks[destination_set_id].fields = source.blocks[source_set_id].fields.clone();
                nif.blocks[destination_set_id].remainder =
                    source.blocks[source_set_id].remainder.clone();
            }
        }
    }
    true
}

fn copy_legacy_geometry(
    nif: &mut NifFile,
    source: &NifFile,
    destination_shape_id: usize,
    source_shape_id: usize,
    removed_tangent_blocks: &mut Vec<usize>,
) -> bool {
    let Some(source_data_id) = value_ref(source.blocks[source_shape_id].get_field("Data"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|reference| *reference < source.blocks.len())
    else {
        return false;
    };
    let Some(destination_data_id) = value_ref(nif.blocks[destination_shape_id].get_field("Data"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|reference| *reference < nif.blocks.len())
    else {
        return false;
    };
    let additional_data = nif.blocks[destination_data_id]
        .get_field("Additional Data")
        .cloned();
    nif.blocks[destination_data_id].fields = source.blocks[source_data_id].fields.clone();
    nif.blocks[destination_data_id].remainder = source.blocks[source_data_id].remainder.clone();
    if let Some(additional_data) = additional_data {
        nif.blocks[destination_data_id].set_field("Additional Data", additional_data);
    }
    if crate::validation::nif_game_label(nif) == "oblivion" {
        copy_oblivion_tangent_data(
            nif,
            source,
            destination_shape_id,
            source_shape_id,
            removed_tangent_blocks,
        );
    }
    true
}

fn copy_oblivion_tangent_data(
    nif: &mut NifFile,
    source: &NifFile,
    destination_shape_id: usize,
    source_shape_id: usize,
    removed_tangent_blocks: &mut Vec<usize>,
) {
    let source_tangent = oblivion_tangent_block(source, source_shape_id);
    let destination_tangent = oblivion_tangent_block(nif, destination_shape_id);
    match (source_tangent, destination_tangent) {
        (Some(source_tangent), Some(destination_tangent)) => {
            nif.blocks[destination_tangent].fields = source.blocks[source_tangent].fields.clone();
            nif.blocks[destination_tangent].remainder =
                source.blocks[source_tangent].remainder.clone();
        }
        (Some(source_tangent), None) => {
            let destination_tangent = copy_block(source, nif, source_tangent);
            append_ref(
                &mut nif.blocks[destination_shape_id],
                "Extra Data List",
                destination_tangent,
            );
        }
        (None, Some(destination_tangent)) => {
            let refs = value_array(nif.blocks[destination_shape_id].get_field("Extra Data List"))
                .iter()
                .filter(|value| value_ref(Some(value)) != Some(destination_tangent as i32))
                .cloned()
                .collect::<Vec<_>>();
            nif.blocks[destination_shape_id]
                .set_field("Num Extra Data List", NifValue::UInt(refs.len() as u64));
            nif.blocks[destination_shape_id].set_field("Extra Data List", NifValue::Array(refs));
            removed_tangent_blocks.push(destination_tangent);
        }
        (None, None) => {}
    }
}

fn copy_modern_geometry(
    nif: &mut NifFile,
    source: &NifFile,
    destination_shape_id: usize,
    source_shape_id: usize,
) {
    let preserved = [
        "Controller",
        "Collision Object",
        "Skin",
        "Shader Property",
        "Alpha Property",
        "Extra Data List",
    ]
    .into_iter()
    .filter_map(|field| {
        nif.blocks[destination_shape_id]
            .get_field(field)
            .cloned()
            .map(|value| (field, value))
    })
    .collect::<Vec<_>>();
    nif.blocks[destination_shape_id].fields = source.blocks[source_shape_id].fields.clone();
    nif.blocks[destination_shape_id].remainder = source.blocks[source_shape_id].remainder.clone();
    for (field, value) in preserved {
        nif.blocks[destination_shape_id].set_field(field, value);
    }
    let extra_data_count =
        value_array(nif.blocks[destination_shape_id].get_field("Extra Data List")).len();
    if nif.blocks[destination_shape_id]
        .get_field("Extra Data List")
        .is_some()
    {
        nif.blocks[destination_shape_id].set_field(
            "Num Extra Data List",
            NifValue::UInt(extra_data_count as u64),
        );
    }
}

fn merge_properties(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let block_types = options
        .get("property_types")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![
                "BSShaderTextureSet".to_string(),
                "NiMaterialProperty".to_string(),
            ]
        });
    if block_types.is_empty() {
        return Err("Select properties to merge".to_string());
    }
    let ignore_name = option_bool(options, "ignore_name").unwrap_or(true);
    let ignore_specular = matches!(
        crate::validation::nif_game_label(nif),
        "fo3/fnv" | "skyrim" | "skyrimse" | "fo4"
    );
    let mut canonical = Vec::<(String, Vec<(String, NifValue)>, usize)>::new();
    let mut replacements = Vec::<(usize, usize)>::new();
    for block in nif.blocks.iter().rev() {
        if !block_types.iter().any(|value| value == &block.type_name) {
            continue;
        }
        let fields = block
            .fields
            .iter()
            .filter(|(name, _)| !(ignore_name && bare_name(name) == "Name"))
            .filter(|(name, _)| {
                !(ignore_specular
                    && block.type_name == "NiMaterialProperty"
                    && bare_name(name) == "Specular Color")
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>();
        if let Some((_, _, target)) = canonical.iter().find(|(block_type, candidate, _)| {
            block_type == &block.type_name && candidate == &fields
        }) {
            replacements.push((block.block_id, *target));
        } else {
            canonical.push((block.type_name.clone(), fields, block.block_id));
        }
    }
    if replacements.is_empty() {
        return Ok(Vec::new());
    }
    let reachable_before = reachable_blocks(nif);
    for block in &mut nif.blocks {
        for value in block.fields.values_mut() {
            replace_block_refs(value, &replacements);
        }
    }
    let reachable_after = reachable_blocks(nif);
    let duplicate_ids = replacements
        .iter()
        .map(|(duplicate, _)| *duplicate)
        .collect::<HashSet<_>>();
    let removed = nif
        .blocks
        .iter()
        .filter(|block| {
            duplicate_ids.contains(&block.block_id)
                || (reachable_before.contains(&block.block_id)
                    && !reachable_after.contains(&block.block_id))
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let changes = replacements
        .iter()
        .map(|(duplicate, target)| format!("{duplicate}: Merged property into {target}"))
        .collect();
    nif.remove_blocks(&removed);
    Ok(changes)
}

fn replace_block_refs(value: &mut NifValue, replacements: &[(usize, usize)]) {
    match value {
        NifValue::Ref(reference) if *reference >= 0 => {
            if let Some((_, target)) = replacements
                .iter()
                .find(|(source, _)| *source == *reference as usize)
            {
                *reference = *target as i32;
            }
        }
        NifValue::Array(values) => {
            for value in values {
                replace_block_refs(value, replacements);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                replace_block_refs(value, replacements);
            }
        }
        _ => {}
    }
}

fn group_shapes(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let Some(mut root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    let split = option_bool(options, "split").unwrap_or(false);
    let all_features = option_bool(options, "all_features").unwrap_or(false);
    let mut groups = IndexMap::<String, Vec<usize>>::new();
    for shape_id in value_array(nif.blocks[root_id].get_field("Children"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
    {
        let Some(shape) = nif.get_block(shape_id) else {
            continue;
        };
        if !SCHEMA.is_subtype_of(&shape.type_name, "NiTriBasedGeom")
            && !SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape")
        {
            continue;
        }
        let token = shape_texture_token(nif, shape_id, all_features).to_lowercase();
        if !token.is_empty() {
            groups.entry(token).or_default().push(shape_id);
        }
    }
    let mut used_names = nif
        .blocks
        .iter()
        .filter_map(|block| block.get_field("Name").and_then(value_string))
        .map(str::to_ascii_lowercase)
        .collect::<HashSet<_>>();
    let mut planned = Vec::<(usize, String, Vec<usize>)>::new();
    for (token, shape_ids) in groups {
        if shape_ids.len() < 2 {
            continue;
        }
        let chunks = if split {
            split_shape_group(nif, &shape_ids)
        } else {
            vec![shape_ids]
        };
        let diffuse = token
            .split(',')
            .next()
            .map(asset_basename)
            .filter(|value| !value.is_empty())
            .unwrap_or("nodiffuse.dds");
        for chunk in chunks {
            let name = unique_name(diffuse, &mut used_names);
            planned.push((chunk[0], name, chunk));
        }
    }
    planned.sort_by_key(|(first, _, _)| std::cmp::Reverse(*first));
    let mut changes = Vec::new();
    for (insertion, name, shape_ids) in planned {
        let node_id = nif.insert_block(insertion, "NiNode");
        root_id += usize::from(root_id >= insertion);
        let shifted_shapes = shape_ids
            .into_iter()
            .map(|shape_id| shape_id + usize::from(shape_id >= insertion))
            .collect::<Vec<_>>();
        nif.blocks[node_id].set_field("Name", NifValue::String(name.clone()));
        nif.blocks[node_id].set_field("Num Children", NifValue::UInt(shifted_shapes.len() as u64));
        nif.blocks[node_id].set_field(
            "Children",
            NifValue::Array(
                shifted_shapes
                    .iter()
                    .map(|shape_id| NifValue::Ref(*shape_id as i32))
                    .collect(),
            ),
        );
        let first_shape = shifted_shapes[0] as i32;
        let grouped = shifted_shapes.iter().copied().collect::<HashSet<_>>();
        let root_children = value_array(nif.blocks[root_id].get_field("Children"))
            .iter()
            .filter_map(|value| {
                let reference = value_ref(Some(value))?;
                if reference == first_shape {
                    Some(NifValue::Ref(node_id as i32))
                } else if reference >= 0 && grouped.contains(&(reference as usize)) {
                    None
                } else {
                    Some(value.clone())
                }
            })
            .collect::<Vec<_>>();
        nif.blocks[root_id].set_field("Num Children", NifValue::UInt(root_children.len() as u64));
        nif.blocks[root_id].set_field("Children", NifValue::Array(root_children));
        changes.push(format!(
            "{node_id}: Grouped {} shapes under {name:?}",
            shifted_shapes.len()
        ));
    }
    changes
}

fn merge_shapes(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let names = options
        .get("names")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err("Names field can not be empty".to_string());
    }
    let exact = option_bool(options, "exact_match").unwrap_or(true);
    let node_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiNode"))
        .filter(|block| {
            let name = block
                .get_field("Name")
                .and_then(value_string)
                .unwrap_or_default()
                .to_ascii_lowercase();
            names.iter().any(|candidate| {
                if exact {
                    name == *candidate
                } else {
                    name.contains(candidate)
                }
            })
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let modern_game = matches!(crate::validation::nif_game_label(nif), "skyrimse" | "fo4");
    let reachable_before = reachable_blocks(nif);
    let mut changes = Vec::new();

    for node_id in node_ids {
        let children = value_array(nif.blocks[node_id].get_field("Children"))
            .iter()
            .filter_map(|value| value_ref(Some(value)))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
            .filter(|child_id| *child_id < nif.blocks.len())
            .collect::<Vec<_>>();
        let shape_ids = children
            .iter()
            .copied()
            .filter(|child_id| {
                if modern_game {
                    nif.blocks[*child_id].type_name == "BSTriShape"
                } else {
                    SCHEMA.is_subtype_of(&nif.blocks[*child_id].type_name, "NiTriBasedGeom")
                }
            })
            .collect::<Vec<_>>();
        if shape_ids.len() < 2 {
            continue;
        }
        let totals = shape_ids
            .iter()
            .fold((0usize, 0usize), |(vertices, tris), shape_id| {
                let data_id = if modern_game {
                    Some(*shape_id)
                } else {
                    value_ref(nif.blocks[*shape_id].get_field("Data"))
                        .filter(|reference| *reference >= 0)
                        .map(|reference| reference as usize)
                };
                let Some(data_id) = data_id.filter(|id| *id < nif.blocks.len()) else {
                    return (vertices, tris);
                };
                (
                    vertices + positions(&nif.blocks[data_id]).len(),
                    tris + triangles(&nif.blocks[data_id]).len(),
                )
            });
        if totals.0 > u16::MAX as usize || totals.1 > u16::MAX as usize {
            continue;
        }

        let target_shape_id = shape_ids[0];
        bake_single_shape_transform(nif, target_shape_id);
        if !modern_game {
            triangulate_legacy_shape(nif, target_shape_id)?;
        }
        let target_data_id = if modern_game {
            target_shape_id
        } else {
            value_ref(nif.blocks[target_shape_id].get_field("Data")).unwrap() as usize
        };
        let mut removed = HashSet::new();
        for source_shape_id in shape_ids.iter().skip(1).copied() {
            bake_single_shape_transform(nif, source_shape_id);
            if !modern_game {
                triangulate_legacy_shape(nif, source_shape_id)?;
            }
            let source_data_id = if modern_game {
                source_shape_id
            } else {
                let Some(reference) = value_ref(nif.blocks[source_shape_id].get_field("Data"))
                    .filter(|reference| *reference >= 0)
                    .map(|reference| reference as usize)
                else {
                    continue;
                };
                reference
            };
            let vertex_offset = positions(&nif.blocks[target_data_id]).len() as u32;
            if modern_game {
                append_array_field(nif, target_data_id, source_data_id, "Vertex Data");
            } else {
                for field in [
                    "Vertices",
                    "Normals",
                    "Tangents",
                    "Bitangents",
                    "Vertex Colors",
                ] {
                    append_array_field(nif, target_data_id, source_data_id, field);
                }
                append_first_nested_array(nif, target_data_id, source_data_id, "UV Sets");
            }
            let mut target_triangles = triangles(&nif.blocks[target_data_id]);
            target_triangles.extend(
                triangles(&nif.blocks[source_data_id])
                    .into_iter()
                    .map(|triangle| triangle.map(|index| index + vertex_offset)),
            );
            set_triangles(&mut nif.blocks[target_data_id], &target_triangles);
            set_geometry_counts(&mut nif.blocks[target_data_id]);
            removed.insert(source_shape_id);
            changes.push(format!(
                "{node_id}: Merged shape {source_shape_id} into {target_shape_id}"
            ));
        }
        if !removed.is_empty() {
            let remaining = value_array(nif.blocks[node_id].get_field("Children"))
                .iter()
                .filter(|value| {
                    value_ref(Some(value))
                        .filter(|reference| *reference >= 0)
                        .is_none_or(|reference| !removed.contains(&(reference as usize)))
                })
                .cloned()
                .collect::<Vec<_>>();
            nif.blocks[node_id].set_field("Num Children", NifValue::UInt(remaining.len() as u64));
            nif.blocks[node_id].set_field("Children", NifValue::Array(remaining));
            let points = positions(&nif.blocks[target_data_id]);
            if !points.is_empty()
                && nif.blocks[target_data_id]
                    .get_field("Bounding Sphere")
                    .is_some()
            {
                nif.blocks[target_data_id].set_field("Bounding Sphere", bounding_sphere(&points));
            }
            if crate::validation::nif_game_label(nif) == "oblivion" {
                update_oblivion_shape_tangents(nif, target_shape_id);
            }
        }
    }
    if !changes.is_empty() {
        let reachable_after = reachable_blocks(nif);
        let orphaned = reachable_before
            .difference(&reachable_after)
            .copied()
            .collect::<Vec<_>>();
        nif.remove_blocks(&orphaned);
    }
    Ok(changes)
}

fn update_oblivion_shape_tangents(nif: &mut NifFile, shape_id: usize) {
    if oblivion_tangent_block(nif, shape_id).is_none() {
        return;
    }
    let Some(data_id) = value_ref(nif.blocks[shape_id].get_field("Data"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|reference| *reference < nif.blocks.len())
    else {
        return;
    };
    let geometry = tangent_geometry(&nif.blocks[data_id]);
    if geometry.positions.is_empty()
        || geometry.normals.len() != geometry.positions.len()
        || geometry.uvs.len() != geometry.positions.len()
        || geometry.triangles.is_empty()
    {
        return;
    }
    let (tangents, bitangents) = recompute_tangents_lengyel(
        &geometry.positions,
        &geometry.normals,
        &geometry.uvs,
        &geometry.triangles,
    );
    if !tangents.is_empty() {
        write_oblivion_tangents(nif, shape_id, &tangents, &bitangents);
    }
}

fn bake_single_shape_transform(nif: &mut NifFile, shape_id: usize) {
    let transform = block_scene_transform(&nif.blocks[shape_id]);
    if scene_transform_is_identity(transform) {
        return;
    }
    if SCHEMA.is_subtype_of(&nif.blocks[shape_id].type_name, "BSTriShape") {
        bake_modern_shape_transform(&mut nif.blocks[shape_id], transform);
    } else if let Some(data_id) = value_ref(nif.blocks[shape_id].get_field("Data"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|reference| *reference < nif.blocks.len())
    {
        bake_shape_data_transform(&mut nif.blocks[data_id], transform);
    }
    set_identity_scene_transform(&mut nif.blocks[shape_id]);
}

fn triangulate_legacy_shape(nif: &mut NifFile, shape_id: usize) -> Result<(), String> {
    let Some(data_id) = value_ref(nif.blocks[shape_id].get_field("Data"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|reference| *reference < nif.blocks.len())
    else {
        return Ok(());
    };
    let triangles = triangles(&nif.blocks[data_id]);
    if nif.blocks[shape_id].type_name == "NiTriStrips" {
        nif.convert_block_type(shape_id, "NiTriShape")?;
    }
    if nif.blocks[data_id].type_name == "NiTriStripsData" {
        nif.convert_block_type(data_id, "NiTriShapeData")?;
    }
    set_triangles(&mut nif.blocks[data_id], &triangles);
    set_geometry_counts(&mut nif.blocks[data_id]);
    Ok(())
}

fn append_array_field(nif: &mut NifFile, target: usize, source: usize, field: &str) {
    let source_values = value_array(nif.blocks[source].get_field(field)).to_vec();
    if source_values.is_empty() || nif.blocks[target].get_field(field).is_none() {
        return;
    }
    if let Some(NifValue::Array(target_values)) = nif.blocks[target].get_field_mut(field) {
        target_values.extend(source_values);
    }
}

fn append_first_nested_array(nif: &mut NifFile, target: usize, source: usize, field: &str) {
    let source_values = value_array(nif.blocks[source].get_field(field))
        .first()
        .and_then(|value| match value {
            NifValue::Array(values) => Some(values.clone()),
            _ => None,
        });
    let Some(source_values) = source_values else {
        return;
    };
    let Some(NifValue::Array(target_sets)) = nif.blocks[target].get_field_mut(field) else {
        return;
    };
    let Some(NifValue::Array(target_values)) = target_sets.first_mut() else {
        return;
    };
    target_values.extend(source_values);
}

fn set_triangles(block: &mut NifBlock, triangles: &[[u32; 3]]) {
    block.set_field(
        "Triangles",
        NifValue::Array(
            triangles
                .iter()
                .map(|triangle| {
                    NifValue::Struct(IndexMap::from([
                        ("v1".to_string(), NifValue::UInt(triangle[0] as u64)),
                        ("v2".to_string(), NifValue::UInt(triangle[1] as u64)),
                        ("v3".to_string(), NifValue::UInt(triangle[2] as u64)),
                    ]))
                })
                .collect(),
        ),
    );
}

fn set_geometry_counts(block: &mut NifBlock) {
    let vertices = positions(block).len();
    let triangles = triangles(block).len();
    block.set_field("Num Vertices", NifValue::UInt(vertices as u64));
    block.set_field("Num Triangles", NifValue::UInt(triangles as u64));
    if block.get_field("Num Triangle Points").is_some() {
        block.set_field(
            "Num Triangle Points",
            NifValue::UInt((triangles * 3) as u64),
        );
    }
}

fn vertex_paint(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let mode = options
        .get("paint_mode")
        .and_then(Value::as_str)
        .unwrap_or("set");
    if !matches!(mode, "set" | "adjust" | "remove" | "replace") {
        return Err(format!("Unknown vertex paint mode: {mode}"));
    }
    let color = parse_hex_color(
        options
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or("FFFFFFFF"),
    )?;
    let replacement = parse_hex_color(
        options
            .get("replacement_color")
            .and_then(Value::as_str)
            .unwrap_or("FFFFFFFF"),
    )?;
    let skip_color = parse_hex_color(
        options
            .get("skip_color")
            .and_then(Value::as_str)
            .unwrap_or("FFFFFFFF"),
    )?;
    let name_filter = options
        .get("shape_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let skip = option_bool(options, "skip_color_enabled").unwrap_or(false);
    let all_white = option_bool(options, "all_white").unwrap_or(false);
    let add_if_missing = option_bool(options, "add_if_missing").unwrap_or(false);
    let adjust_mode = options
        .get("adjust_mode")
        .and_then(Value::as_str)
        .unwrap_or("multiply");
    if !matches!(adjust_mode, "multiply" | "add") {
        return Err(format!("Unknown color adjustment mode: {adjust_mode}"));
    }
    let adjust_default = if adjust_mode == "multiply" { 1.0 } else { 0.0 };
    let adjustments = ["adjust_h", "adjust_s", "adjust_l", "adjust_a"]
        .map(|name| option_f64(options, name).unwrap_or(adjust_default));
    if mode == "adjust"
        && ["adjust_h", "adjust_s", "adjust_l", "adjust_a"]
            .iter()
            .all(|name| options.get(*name).is_none_or(Value::is_null))
    {
        return Err("At least one color adjustment is required".to_string());
    }

    let skyrimse = crate::validation::nif_game_label(nif) == "skyrimse";
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom")
                || SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
                || (skyrimse && block.type_name == "NiSkinPartition")
        })
        .filter(|block| {
            name_filter.is_empty()
                || block
                    .get_field("Name")
                    .and_then(value_string)
                    .is_some_and(|name| name.to_ascii_lowercase().contains(&name_filter))
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();

    for shape_id in shape_ids {
        let modern = SCHEMA.is_subtype_of(&nif.blocks[shape_id].type_name, "BSTriShape")
            || nif.blocks[shape_id].type_name == "NiSkinPartition";
        let data_id = if modern {
            shape_id
        } else {
            let Some(reference) = value_ref(nif.blocks[shape_id].get_field("Data"))
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize)
                .filter(|reference| *reference < nif.blocks.len())
            else {
                continue;
            };
            reference
        };
        if mode == "remove" && !vertex_colors_may_be_removed(nif, shape_id, modern) {
            continue;
        }
        let has_colors = if modern {
            vertex_desc_flags(&nif.blocks[data_id]) & 0x20 != 0
        } else {
            option_value_bool(nif.blocks[data_id].get_field("Has Vertex Colors")).unwrap_or(false)
        };
        if modern && mode != "set" && !has_colors {
            continue;
        }
        if mode == "set" && !has_colors {
            if modern {
                if !add_if_missing {
                    continue;
                }
                let descriptor =
                    value_u64(nif.blocks[data_id].get_field("Vertex Desc")).unwrap_or(0);
                nif.blocks[data_id].set_field(
                    "Vertex Desc",
                    NifValue::UInt(insert_vertex_attribute(descriptor, 0x20, 24, &[28, 32, 36])),
                );
            } else if add_if_missing {
                nif.blocks[data_id].set_field("Has Vertex Colors", NifValue::Bool(true));
            }
        }
        if mode == "set"
            && !modern
            && value_array(nif.blocks[data_id].get_field("Vertex Colors")).is_empty()
            && nif.blocks[data_id].get_field("Vertex Colors").is_some()
        {
            let count = value_array(nif.blocks[data_id].get_field("Vertices")).len();
            nif.blocks[data_id].set_field(
                "Vertex Colors",
                NifValue::Array(vec![NifValue::Color4([1.0; 4]); count]),
            );
        }

        if mode == "remove" {
            let colors = vertex_colors(&nif.blocks[data_id], modern);
            if all_white && colors.iter().any(|existing| !same_color(*existing, color)) {
                continue;
            }
            if modern {
                let descriptor =
                    value_u64(nif.blocks[data_id].get_field("Vertex Desc")).unwrap_or(0);
                nif.blocks[data_id]
                    .set_field("Vertex Desc", NifValue::UInt(descriptor & !(0x20 << 44)));
            } else {
                nif.blocks[data_id].set_field("Has Vertex Colors", NifValue::Bool(false));
            }
            changes.push(format!("{shape_id}: Removed vertex colors"));
            continue;
        }

        let mut changed_count = 0usize;
        if modern {
            if let Some(NifValue::Array(entries)) = nif.blocks[data_id].get_field_mut("Vertex Data")
            {
                for entry in entries {
                    let NifValue::Struct(fields) = entry else {
                        continue;
                    };
                    let existing = named_value(fields, "Vertex Colors")
                        .and_then(|value| read_vertex_color(value, true))
                        .unwrap_or([1.0; 4]);
                    if skip && same_color(existing, skip_color) {
                        continue;
                    }
                    let next =
                        paint_color(existing, mode, color, replacement, adjustments, adjust_mode);
                    if !same_color(existing, next) {
                        set_named_color(fields, "Vertex Colors", next, true);
                        changed_count += 1;
                    }
                }
            }
        } else if let Some(NifValue::Array(colors)) =
            nif.blocks[data_id].get_field_mut("Vertex Colors")
        {
            for entry in colors {
                let Some(existing) = read_vertex_color(entry, false) else {
                    continue;
                };
                if skip && same_color(existing, skip_color) {
                    continue;
                }
                let next =
                    paint_color(existing, mode, color, replacement, adjustments, adjust_mode);
                if !same_color(existing, next) {
                    write_vertex_color(entry, next, false);
                    changed_count += 1;
                }
            }
        }
        if changed_count > 0 {
            changes.push(format!(
                "{shape_id}: Painted {changed_count} vertex color(s)"
            ));
        }
    }
    Ok(changes)
}

fn option_value_bool(value: Option<&NifValue>) -> Option<bool> {
    match value? {
        NifValue::Bool(value) => Some(*value),
        NifValue::Int(value) => Some(*value != 0),
        NifValue::UInt(value) => Some(*value != 0),
        _ => None,
    }
}

fn parse_hex_color(value: &str) -> Result<[f64; 4], String> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 8 {
        return Err(format!(
            "Color must contain 8 hexadecimal digits: {value:?}"
        ));
    }
    let packed = u32::from_str_radix(value, 16)
        .map_err(|_| format!("Color is not a valid hexadecimal number: {value:?}"))?;
    Ok([
        ((packed >> 24) & 0xff) as f64 / 255.0,
        ((packed >> 16) & 0xff) as f64 / 255.0,
        ((packed >> 8) & 0xff) as f64 / 255.0,
        (packed & 0xff) as f64 / 255.0,
    ])
}

fn vertex_colors(block: &NifBlock, modern: bool) -> Vec<[f64; 4]> {
    if modern {
        value_array(block.get_field("Vertex Data"))
            .iter()
            .filter_map(|entry| {
                nested_value(Some(entry), "Vertex Colors")
                    .and_then(|value| read_vertex_color(value, true))
            })
            .collect()
    } else {
        value_array(block.get_field("Vertex Colors"))
            .iter()
            .filter_map(|value| read_vertex_color(value, false))
            .collect()
    }
}

fn read_vertex_color(value: &NifValue, byte_color: bool) -> Option<[f64; 4]> {
    match value {
        NifValue::Color4(color) | NifValue::Vec4(color) => Some(color.map(|value| value as f64)),
        NifValue::Bytes(bytes) if bytes.len() >= 4 => Some([
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            bytes[3] as f64 / 255.0,
        ]),
        NifValue::Struct(fields) => {
            let divisor = if byte_color { 255.0 } else { 1.0 };
            Some([
                value_f64(Some(named_value(fields, "r")?))? / divisor,
                value_f64(Some(named_value(fields, "g")?))? / divisor,
                value_f64(Some(named_value(fields, "b")?))? / divisor,
                value_f64(Some(named_value(fields, "a")?))? / divisor,
            ])
        }
        _ => None,
    }
}

fn write_vertex_color(value: &mut NifValue, color: [f64; 4], byte_color: bool) {
    match value {
        NifValue::Color4(existing) | NifValue::Vec4(existing) => {
            *existing = color.map(|value| value as f32);
        }
        NifValue::Bytes(existing) if existing.len() >= 4 => {
            for (component, value) in existing.iter_mut().take(4).zip(color) {
                *component = color_byte(value);
            }
        }
        NifValue::Struct(fields) => {
            for (name, component) in ["r", "g", "b", "a"].into_iter().zip(color) {
                let replacement = if byte_color {
                    NifValue::UInt(color_byte(component) as u64)
                } else {
                    NifValue::Float(component)
                };
                if let Some(value) = named_value_mut(fields, name) {
                    *value = replacement;
                } else {
                    fields.insert(name.to_string(), replacement);
                }
            }
        }
        _ => {
            *value = if byte_color {
                NifValue::Struct(IndexMap::from([
                    ("r".to_string(), NifValue::UInt(color_byte(color[0]) as u64)),
                    ("g".to_string(), NifValue::UInt(color_byte(color[1]) as u64)),
                    ("b".to_string(), NifValue::UInt(color_byte(color[2]) as u64)),
                    ("a".to_string(), NifValue::UInt(color_byte(color[3]) as u64)),
                ]))
            } else {
                NifValue::Color4(color.map(|value| value as f32))
            };
        }
    }
}

fn set_named_color(
    fields: &mut IndexMap<String, NifValue>,
    name: &str,
    color: [f64; 4],
    byte_color: bool,
) {
    if let Some(value) = named_value_mut(fields, name) {
        write_vertex_color(value, color, byte_color);
    } else {
        let mut value = NifValue::Null;
        write_vertex_color(&mut value, color, byte_color);
        fields.insert(name.to_string(), value);
    }
}

fn paint_color(
    existing: [f64; 4],
    mode: &str,
    color: [f64; 4],
    replacement: [f64; 4],
    adjustments: [f64; 4],
    adjust_mode: &str,
) -> [f64; 4] {
    match mode {
        "set" => color,
        "replace" if same_color(existing, color) => replacement,
        "replace" => existing,
        "adjust" => adjust_vertex_color(existing, adjustments, adjust_mode),
        _ => existing,
    }
}

fn adjust_vertex_color(color: [f64; 4], adjustments: [f64; 4], mode: &str) -> [f64; 4] {
    let [mut h, mut s, mut l] = rgb_to_hsl(color[0], color[1], color[2]);
    let mut a = color[3];
    if mode == "multiply" {
        h *= adjustments[0];
        s *= adjustments[1];
        l *= adjustments[2];
        a *= adjustments[3];
    } else {
        h += adjustments[0];
        s += adjustments[1];
        l += adjustments[2];
        a += adjustments[3];
    }
    let [r, g, b] = hsl_to_rgb(h.clamp(0.0, 1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    [r, g, b, a.clamp(0.0, 1.0)]
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> [f64; 3] {
    let maximum = r.max(g).max(b);
    let minimum = r.min(g).min(b);
    let l = (maximum + minimum) / 2.0;
    if maximum == minimum {
        return [0.0, 0.0, l];
    }
    let delta = maximum - minimum;
    let s = if l > 0.5 {
        delta / (2.0 - maximum - minimum)
    } else {
        delta / (maximum + minimum)
    };
    let h = if maximum == r {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if maximum == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    [h / 6.0, s, l]
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    [
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    ]
}

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

fn color_byte(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn same_color(left: [f64; 4], right: [f64; 4]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| color_byte(left) == color_byte(right))
}

fn vertex_colors_may_be_removed(nif: &NifFile, shape_id: usize, modern: bool) -> bool {
    if modern && nif.blocks[shape_id].type_name == "NiSkinPartition" {
        return false;
    }
    if modern
        && value_ref(nif.blocks[shape_id].get_field("Skin")).is_some_and(|reference| reference >= 0)
    {
        return false;
    }
    if !matches!(
        crate::validation::nif_game_label(nif),
        "skyrim" | "skyrimse"
    ) {
        return true;
    }
    let Some(shader_id) = geometry_shader_id(nif, shape_id) else {
        return false;
    };
    let shader = &nif.blocks[shader_id];
    shader.type_name != "BSLightingShaderProperty"
        || (value_u64(shader.get_field("Shader Type")) != Some(3)
            && value_u64(shader.get_field("Shader Flags 2"))
                .is_none_or(|flags| flags & (1 << 29) == 0))
}

#[derive(Clone, Copy)]
struct SceneTransform {
    translation: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: f32,
}

impl SceneTransform {
    fn identity() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
        }
    }
}

fn add_root_collision_node(nif: &mut NifFile) -> Vec<String> {
    let Some(root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    if nif.blocks[root_id].type_name != "NiNode"
        || nif
            .blocks
            .iter()
            .any(|block| block.type_name == "RootCollisionNode")
    {
        return Vec::new();
    }
    let mut shape_data = Vec::<(usize, SceneTransform)>::new();
    let mut seen_data = HashSet::new();
    collect_collision_shape_data(
        nif,
        root_id,
        root_id,
        SceneTransform::identity(),
        &mut seen_data,
        &mut shape_data,
    );
    if shape_data.is_empty() {
        return Vec::new();
    }
    let collision_id = nif.add_block("RootCollisionNode", None);
    nif.blocks[collision_id].set_field("Name", NifValue::String("RCN".to_string()));
    nif.blocks[collision_id].set_field("Flags", NifValue::UInt(3));
    append_ref(&mut nif.blocks[root_id], "Children", collision_id);
    let mut collision_shapes = Vec::new();
    for (source_data_id, transform) in shape_data {
        let source_fields = nif.blocks[source_data_id].fields.clone();
        let shape_id = nif.add_block("NiTriShape", None);
        nif.blocks[shape_id].set_field("Flags", NifValue::UInt(2));
        let data_id = nif.add_block("NiTriShapeData", Some(source_fields));
        nif.blocks[shape_id].set_field("Data", NifValue::Ref(data_id as i32));
        strip_collision_shape_data(&mut nif.blocks[data_id]);
        bake_shape_data_transform(&mut nif.blocks[data_id], transform);
        collision_shapes.push(NifValue::Ref(shape_id as i32));
    }
    nif.blocks[collision_id].set_field(
        "Num Children",
        NifValue::UInt(collision_shapes.len() as u64),
    );
    nif.blocks[collision_id].set_field("Children", NifValue::Array(collision_shapes));
    vec![format!(
        "{collision_id}: Added RootCollisionNode with {} shapes",
        value_array(nif.blocks[collision_id].get_field("Children")).len()
    )]
}

fn apply_transforms(nif: &mut NifFile, options: &Value) -> Vec<String> {
    let Some(root_id) = roots(nif).first().copied() else {
        return Vec::new();
    };
    let apply_skinned = option_bool(options, "apply_skinned").unwrap_or(false);
    let apply_animated = option_bool(options, "apply_animated").unwrap_or(false);
    let apply_collision = option_bool(options, "apply_collision").unwrap_or(false);
    let apply_root = option_bool(options, "apply_root").unwrap_or(false);
    let apply_controller_manager =
        option_bool(options, "apply_controller_manager").unwrap_or(false);
    let has_controller_manager = nif
        .blocks
        .iter()
        .any(|block| block.type_name == "NiControllerManager");
    let has_skin_instances = nif.blocks.iter().any(|block| {
        matches!(
            block.type_name.as_str(),
            "NiSkinInstance" | "BSSkin::Instance"
        )
    });
    let mut changes = Vec::new();
    let mut visited = HashSet::new();
    apply_transform_recursive(
        nif,
        root_id,
        root_id,
        apply_skinned,
        apply_animated,
        apply_collision,
        apply_root,
        apply_controller_manager,
        has_controller_manager,
        has_skin_instances,
        &mut visited,
        &mut changes,
    );
    changes
}

#[allow(clippy::too_many_arguments)]
fn apply_transform_recursive(
    nif: &mut NifFile,
    block_id: usize,
    root_id: usize,
    apply_skinned: bool,
    apply_animated: bool,
    apply_collision: bool,
    apply_root: bool,
    apply_controller_manager: bool,
    has_controller_manager: bool,
    has_skin_instances: bool,
    visited: &mut HashSet<usize>,
    changes: &mut Vec<String>,
) {
    if !visited.insert(block_id) || block_id >= nif.blocks.len() {
        return;
    }
    let block_type = nif.blocks[block_id].type_name.clone();
    if block_type == "NiBillboardNode" {
        return;
    }
    let can_transform = can_apply_block_transform(
        nif,
        block_id,
        root_id,
        apply_skinned,
        apply_animated,
        apply_collision,
        apply_root,
        apply_controller_manager,
        has_controller_manager,
        has_skin_instances,
    );
    if SCHEMA.is_subtype_of(&block_type, "NiTriBasedGeom") {
        if !can_transform {
            return;
        }
        let transform = block_scene_transform(&nif.blocks[block_id]);
        if scene_transform_is_identity(transform) {
            return;
        }
        let Some(data_id) = value_ref(nif.blocks[block_id].get_field("Data"))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
        else {
            return;
        };
        if let Some(data) = nif.blocks.get_mut(data_id) {
            bake_shape_data_transform(data, transform);
            set_identity_scene_transform(&mut nif.blocks[block_id]);
            changes.push(format!("{block_id}: Applied transformation"));
        }
        return;
    }
    if SCHEMA.is_subtype_of(&block_type, "BSTriShape") {
        if !can_transform {
            return;
        }
        let skinned_without_vertices =
            value_u64(nif.blocks[block_id].get_field("Num Vertices")).unwrap_or(0) == 0
                && block_has_skin(&nif.blocks[block_id]);
        let transform = block_scene_transform(&nif.blocks[block_id]);
        if skinned_without_vertices || scene_transform_is_identity(transform) {
            return;
        }
        bake_modern_shape_transform(&mut nif.blocks[block_id], transform);
        set_identity_scene_transform(&mut nif.blocks[block_id]);
        changes.push(format!("{block_id}: Applied transformation"));
        return;
    }
    if !SCHEMA.is_subtype_of(&block_type, "NiNode") {
        return;
    }
    let children = value_array(nif.blocks[block_id].get_field("Children"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .collect::<Vec<_>>();
    let child_is_animated = !apply_animated
        && children.iter().any(|child_id| {
            nif.get_block(*child_id)
                .is_some_and(|child| has_valid_ref(child.get_field("Controller")))
        });
    let transform = block_scene_transform(&nif.blocks[block_id]);
    let can_propagate =
        can_transform && !child_is_animated && !scene_transform_is_identity(transform);
    let mut propagated = false;
    for child_id in children {
        if child_id >= nif.blocks.len() {
            continue;
        }
        if can_propagate && SCHEMA.is_subtype_of(&nif.blocks[child_id].type_name, "NiAVObject") {
            let child_transform = block_scene_transform(&nif.blocks[child_id]);
            let combined = compose_scene_transform(transform, child_transform);
            set_scene_transform(&mut nif.blocks[child_id], combined);
            propagated = true;
        }
        apply_transform_recursive(
            nif,
            child_id,
            root_id,
            apply_skinned,
            apply_animated,
            apply_collision,
            apply_root,
            apply_controller_manager,
            has_controller_manager,
            has_skin_instances,
            visited,
            changes,
        );
    }
    if propagated {
        set_identity_scene_transform(&mut nif.blocks[block_id]);
        changes.push(format!("{block_id}: Applied transformation to children"));
    }
}

#[allow(clippy::too_many_arguments)]
fn can_apply_block_transform(
    nif: &NifFile,
    block_id: usize,
    root_id: usize,
    apply_skinned: bool,
    apply_animated: bool,
    apply_collision: bool,
    apply_root: bool,
    apply_controller_manager: bool,
    has_controller_manager: bool,
    has_skin_instances: bool,
) -> bool {
    let block = &nif.blocks[block_id];
    if !apply_skinned && (block_has_skin(block) || block_is_bone(nif, block_id)) {
        return false;
    }
    if !apply_animated && has_valid_ref(block.get_field("Controller")) {
        return false;
    }
    if !apply_collision && has_valid_ref(block.get_field("Collision Object")) {
        return false;
    }
    if !apply_controller_manager && has_controller_manager {
        return false;
    }
    block_id != root_id || apply_root || !has_skin_instances
}

fn block_has_skin(block: &NifBlock) -> bool {
    ["Skin", "Skin Instance"]
        .iter()
        .any(|field| has_valid_ref(block.get_field(field)))
}

fn has_valid_ref(value: Option<&NifValue>) -> bool {
    value_ref(value).is_some_and(|reference| reference >= 0)
}

fn block_is_bone(nif: &NifFile, block_id: usize) -> bool {
    nif.blocks.iter().any(|block| {
        block
            .get_all_ref_fields(&SCHEMA)
            .into_iter()
            .filter(|(field, _)| bare_name(field).starts_with("Bones"))
            .any(|(_, references)| references.contains(&(block_id as i32)))
    })
}

fn scene_transform_is_identity(transform: SceneTransform) -> bool {
    transform.translation == [0.0; 3]
        && transform.rotation == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        && transform.scale == 1.0
}

fn set_scene_transform(block: &mut NifBlock, transform: SceneTransform) {
    if block.get_field("Translation").is_some() {
        block.set_field("Translation", NifValue::Vec3(transform.translation));
    }
    if block.get_field("Rotation").is_some() {
        block.set_field("Rotation", NifValue::Matrix33(transform.rotation));
    }
    if block.get_field("Scale").is_some() {
        block.set_field("Scale", NifValue::Float(transform.scale as f64));
    }
}

fn set_identity_scene_transform(block: &mut NifBlock) {
    set_scene_transform(block, SceneTransform::identity());
}

fn bake_modern_shape_transform(shape: &mut NifBlock, transform: SceneTransform) {
    let mut transformed_positions = Vec::new();
    if let Some(NifValue::Array(vertices)) = shape.get_field_mut("Vertex Data") {
        for vertex in vertices {
            if let Some(position_value) = nested_value_mut(Some(vertex), "Vertex") {
                if let Some(position) = vec3_value(position_value) {
                    let scaled = [
                        position[0] * transform.scale,
                        position[1] * transform.scale,
                        position[2] * transform.scale,
                    ];
                    let rotated = rotate_vec3(transform.rotation, scaled);
                    let transformed = [
                        rotated[0] + transform.translation[0],
                        rotated[1] + transform.translation[1],
                        rotated[2] + transform.translation[2],
                    ];
                    write_vec3_value(position_value, transformed);
                    transformed_positions.push(transformed);
                }
            }
            for field in ["Normal", "Tangent"] {
                if let Some(value) = nested_value_mut(Some(vertex), field) {
                    if let Some(vector) = vec3_value(value) {
                        write_vec3_value(value, rotate_vec3(transform.rotation, vector));
                    }
                }
            }
            let bitangent = ["Bitangent X", "Bitangent Y", "Bitangent Z"]
                .iter()
                .map(|field| {
                    nested_value(Some(vertex), field).and_then(|value| value_f64(Some(value)))
                })
                .collect::<Option<Vec<_>>>();
            if let Some(bitangent) = bitangent {
                let rotated = rotate_vec3(
                    transform.rotation,
                    [
                        bitangent[0] as f32,
                        bitangent[1] as f32,
                        bitangent[2] as f32,
                    ],
                );
                for (field, component) in ["Bitangent X", "Bitangent Y", "Bitangent Z"]
                    .into_iter()
                    .zip(rotated)
                {
                    if let Some(value) = nested_value_mut(Some(vertex), field) {
                        *value = NifValue::Float(component as f64);
                    }
                }
            }
        }
    }
    if !transformed_positions.is_empty() && shape.get_field("Bounding Sphere").is_some() {
        shape.set_field("Bounding Sphere", bounding_sphere(&transformed_positions));
    }
}

fn collect_collision_shape_data(
    nif: &NifFile,
    block_id: usize,
    root_id: usize,
    parent_transform: SceneTransform,
    seen_data: &mut HashSet<usize>,
    output: &mut Vec<(usize, SceneTransform)>,
) {
    let Some(block) = nif.get_block(block_id) else {
        return;
    };
    let transform = if block_id == root_id {
        parent_transform
    } else {
        compose_scene_transform(parent_transform, block_scene_transform(block))
    };
    if block.type_name == "NiTriShape" {
        if value_u64(block.get_field("Flags")).unwrap_or(0) & 1 != 0 {
            return;
        }
        let Some(data_id) = value_ref(block.get_field("Data"))
            .filter(|reference| *reference >= 0)
            .map(|reference| reference as usize)
        else {
            return;
        };
        let Some(data) = nif.get_block(data_id) else {
            return;
        };
        if value_u64(data.get_field("Num Vertices")).unwrap_or(0) == 0
            || value_u64(data.get_field("Num Triangles")).unwrap_or(0) == 0
            || !seen_data.insert(data_id)
        {
            return;
        }
        output.push((data_id, transform));
        return;
    }
    for child_id in value_array(block.get_field("Children"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
    {
        collect_collision_shape_data(nif, child_id, root_id, transform, seen_data, output);
    }
}

fn block_scene_transform(block: &NifBlock) -> SceneTransform {
    SceneTransform {
        translation: block
            .get_field("Translation")
            .and_then(vec3_value)
            .unwrap_or([0.0; 3]),
        rotation: match block.get_field("Rotation") {
            Some(NifValue::Matrix33(rotation)) => *rotation,
            Some(NifValue::Struct(fields)) => [
                [
                    named_value(fields, "m11")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(1.0) as f32,
                    named_value(fields, "m21")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(0.0) as f32,
                    named_value(fields, "m31")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(0.0) as f32,
                ],
                [
                    named_value(fields, "m12")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(0.0) as f32,
                    named_value(fields, "m22")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(1.0) as f32,
                    named_value(fields, "m32")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(0.0) as f32,
                ],
                [
                    named_value(fields, "m13")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(0.0) as f32,
                    named_value(fields, "m23")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(0.0) as f32,
                    named_value(fields, "m33")
                        .and_then(|value| value_f64(Some(value)))
                        .unwrap_or(1.0) as f32,
                ],
            ],
            _ => SceneTransform::identity().rotation,
        },
        scale: value_f64(block.get_field("Scale")).unwrap_or(1.0) as f32,
    }
}

fn compose_scene_transform(parent: SceneTransform, local: SceneTransform) -> SceneTransform {
    let translated = rotate_vec3(
        parent.rotation,
        [
            local.translation[0] * parent.scale,
            local.translation[1] * parent.scale,
            local.translation[2] * parent.scale,
        ],
    );
    SceneTransform {
        translation: [
            parent.translation[0] + translated[0],
            parent.translation[1] + translated[1],
            parent.translation[2] + translated[2],
        ],
        rotation: multiply_matrix33(parent.rotation, local.rotation),
        scale: parent.scale * local.scale,
    }
}

fn multiply_matrix33(left: [[f32; 3]; 3], right: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut result = [[0.0; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            result[row][column] = (0..3)
                .map(|index| left[row][index] * right[index][column])
                .sum();
        }
    }
    result
}

fn rotate_vec3(rotation: [[f32; 3]; 3], value: [f32; 3]) -> [f32; 3] {
    [
        rotation[0][0] * value[0] + rotation[0][1] * value[1] + rotation[0][2] * value[2],
        rotation[1][0] * value[0] + rotation[1][1] * value[1] + rotation[1][2] * value[2],
        rotation[2][0] * value[0] + rotation[2][1] * value[1] + rotation[2][2] * value[2],
    ]
}

fn strip_collision_shape_data(data: &mut NifBlock) {
    for field in ["Num UV Sets", "Num Match Groups"] {
        if data.get_field(field).is_some() {
            data.set_field(field, NifValue::UInt(0));
        }
    }
    for field in ["UV Sets", "Match Groups"] {
        if data.get_field(field).is_some() {
            data.set_field(field, NifValue::Array(Vec::new()));
        }
    }
    for field in ["Has UV", "Has Vertex Colors"] {
        if data.get_field(field).is_some() {
            data.set_field(field, NifValue::Bool(false));
        }
    }
    if data.get_field("Vertex Colors").is_some() {
        data.set_field("Vertex Colors", NifValue::Array(Vec::new()));
    }
}

fn bake_shape_data_transform(data: &mut NifBlock, transform: SceneTransform) {
    let mut transformed_positions = Vec::new();
    if let Some(NifValue::Array(vertices)) = data.get_field_mut("Vertices") {
        for vertex in vertices {
            let Some(position) = vec3_value(vertex) else {
                continue;
            };
            let scaled = [
                position[0] * transform.scale,
                position[1] * transform.scale,
                position[2] * transform.scale,
            ];
            let rotated = rotate_vec3(transform.rotation, scaled);
            let transformed = [
                rotated[0] + transform.translation[0],
                rotated[1] + transform.translation[1],
                rotated[2] + transform.translation[2],
            ];
            write_vec3_value(vertex, transformed);
            transformed_positions.push(transformed);
        }
    }
    for field in ["Normals", "Tangents", "Bitangents"] {
        if let Some(NifValue::Array(values)) = data.get_field_mut(field) {
            for value in values {
                if let Some(vector) = vec3_value(value) {
                    write_vec3_value(value, rotate_vec3(transform.rotation, vector));
                }
            }
        }
    }
    if !transformed_positions.is_empty() && data.get_field("Bounding Sphere").is_some() {
        data.set_field("Bounding Sphere", bounding_sphere(&transformed_positions));
    }
}

fn write_vec3_value(value: &mut NifValue, replacement: [f32; 3]) {
    match value {
        NifValue::Vec3(value) | NifValue::Color3(value) => *value = replacement,
        NifValue::Vec4(value) | NifValue::Quaternion(value) => {
            value[..3].copy_from_slice(&replacement)
        }
        NifValue::Struct(fields) => {
            for (name, component) in ["x", "y", "z"].into_iter().zip(replacement) {
                if let Some(field) = named_value_mut(fields, name) {
                    *field = NifValue::Float(component as f64);
                }
            }
        }
        _ => {}
    }
}

fn split_shape_group(nif: &NifFile, shape_ids: &[usize]) -> Vec<Vec<usize>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut vertices = 0u64;
    let mut triangles = 0u64;
    for shape_id in shape_ids {
        let (shape_vertices, shape_triangles) = shape_counts(nif, *shape_id);
        if !current.is_empty()
            && (vertices + shape_vertices > u16::MAX as u64
                || triangles + shape_triangles > u16::MAX as u64)
        {
            chunks.push(std::mem::take(&mut current));
            vertices = 0;
            triangles = 0;
        }
        current.push(*shape_id);
        vertices += shape_vertices;
        triangles += shape_triangles;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn shape_counts(nif: &NifFile, shape_id: usize) -> (u64, u64) {
    let Some(shape) = nif.get_block(shape_id) else {
        return (0, 0);
    };
    if SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape") {
        return (
            value_u64(shape.get_field("Num Vertices")).unwrap_or(0),
            value_u64(shape.get_field("Num Triangles")).unwrap_or(0),
        );
    }
    value_ref(shape.get_field("Data"))
        .filter(|reference| *reference >= 0)
        .and_then(|reference| nif.get_block(reference as usize))
        .map(|data| {
            (
                value_u64(data.get_field("Num Vertices")).unwrap_or(0),
                value_u64(data.get_field("Num Triangles")).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0))
}

fn shape_texture_token(nif: &NifFile, shape_id: usize, all_features: bool) -> String {
    let Some(shape) = nif.get_block(shape_id) else {
        return String::new();
    };
    if let Some(shader_id) = value_ref(shape.get_field("Shader Property"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
    {
        return shader_texture_token(nif, shader_id, all_features);
    }
    for property in value_array(shape.get_field("Properties"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .filter_map(|reference| nif.get_block(reference as usize))
    {
        if property.type_name == "NiTexturingProperty" {
            let source = property
                .get_field("Base Texture")
                .and_then(|value| nested_value(Some(value), "Source"))
                .or_else(|| property.get_field("Base Texture Source"));
            if let Some(texture) = referenced_block(nif, source)
                .and_then(|block| block.get_field("File Name"))
                .and_then(value_string)
            {
                return asset_basename(texture).to_string();
            }
        } else if property.type_name == "BSShaderPPLightingProperty" {
            return shader_texture_token(nif, property.block_id, all_features);
        }
    }
    String::new()
}

fn shader_texture_token(nif: &NifFile, shader_id: usize, all_features: bool) -> String {
    let Some(shader) = nif.get_block(shader_id) else {
        return String::new();
    };
    if crate::validation::nif_game_label(nif) == "fo4"
        && shader.type_name == "BSLightingShaderProperty"
    {
        if let Some(material) = shader.get_field("Name").and_then(value_string) {
            if !material.is_empty() {
                return asset_basename(material).to_string();
            }
        }
    }
    let Some(texture_set) = referenced_block(nif, shader.get_field("Texture Set")) else {
        return String::new();
    };
    value_array(texture_set.get_field("Textures"))
        .iter()
        .filter_map(value_string)
        .take(if all_features { usize::MAX } else { 1 })
        .map(asset_basename)
        .collect::<Vec<_>>()
        .join(",")
}

fn asset_basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn matrix_to_euler_degrees(matrix: [[f32; 3]; 3]) -> [f64; 3] {
    let m = matrix.map(|row| row.map(f64::from));
    let (x, y, z) = if m[0][2] < 1.0 {
        if m[0][2] > -1.0 {
            (
                (-m[1][2]).atan2(m[2][2]),
                m[0][2].asin(),
                (-m[0][1]).atan2(m[0][0]),
            )
        } else {
            (
                -(-m[1][0]).atan2(m[1][1]),
                -std::f64::consts::FRAC_PI_2,
                0.0,
            )
        }
    } else {
        (m[1][0].atan2(m[1][1]), std::f64::consts::FRAC_PI_2, 0.0)
    };
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

fn euler_degrees_to_matrix(euler: [f64; 3]) -> [[f32; 3]; 3] {
    let [x, y, z] = euler.map(f64::to_radians);
    let (sin_x, cos_x) = x.sin_cos();
    let (sin_y, cos_y) = y.sin_cos();
    let (sin_z, cos_z) = z.sin_cos();
    [
        [
            (cos_y * cos_z) as f32,
            (-cos_y * sin_z) as f32,
            sin_y as f32,
        ],
        [
            (sin_x * sin_y * cos_z + sin_z * cos_x) as f32,
            (cos_x * cos_z - sin_x * sin_y * sin_z) as f32,
            (-sin_x * cos_y) as f32,
        ],
        [
            (sin_x * sin_z - cos_x * sin_y * cos_z) as f32,
            (cos_x * sin_y * sin_z + sin_x * cos_z) as f32,
            (cos_x * cos_y) as f32,
        ],
    ]
}

fn append_ref(block: &mut NifBlock, field: &str, block_id: usize) {
    let mut refs = value_array(block.get_field(field)).to_vec();
    refs.push(NifValue::Ref(block_id as i32));
    block.set_field(
        format!("Num {field}").as_str(),
        NifValue::UInt(refs.len() as u64),
    );
    block.set_field(field, NifValue::Array(refs));
}

fn add_lod_node(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let Some(mut root_id) = roots(nif).first().copied() else {
        return Ok(Vec::new());
    };
    if nif.blocks[root_id].type_name != "BSFadeNode" {
        return Ok(Vec::new());
    }
    let root_children = value_array(nif.blocks[root_id].get_field("Children"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .collect::<Vec<_>>();
    let mut lod_id = root_children.iter().copied().find(|child_id| {
        nif.get_block(*child_id)
            .is_some_and(|child| child.type_name == "NiLODNode")
    });
    let mut shape_ids = root_children
        .iter()
        .copied()
        .filter(|child_id| {
            nif.get_block(*child_id)
                .is_some_and(|child| SCHEMA.is_subtype_of(&child.type_name, "NiTriBasedGeom"))
        })
        .collect::<Vec<_>>();
    if shape_ids.len() < 2 && lod_id.is_none() {
        return Ok(Vec::new());
    }
    let mut changes = Vec::new();
    if lod_id.is_none() {
        let insertion = shape_ids[0];
        nif.insert_block(insertion, "NiLODNode");
        root_id += usize::from(root_id >= insertion);
        for shape_id in &mut shape_ids {
            *shape_id += usize::from(*shape_id >= insertion);
        }
        lod_id = Some(insertion);
        append_ref(&mut nif.blocks[root_id], "Children", insertion);
        changes.push(format!("{insertion}: Added NiLODNode"));
    }
    let lod_id = lod_id.unwrap();
    let data_type = match options
        .get("lod_data")
        .and_then(Value::as_str)
        .unwrap_or("range")
    {
        "range" | "NiRangeLODData" => "NiRangeLODData",
        "screen" | "NiScreenLODData" => "NiScreenLODData",
        value => return Err(format!("Invalid LOD data type: {value}")),
    };
    let mut data_id = value_ref(nif.blocks[lod_id].get_field("LOD Level Data"))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .filter(|reference| *reference < nif.blocks.len());
    if let Some(existing) = data_id {
        if nif.blocks[existing].type_name != data_type {
            nif.convert_block_type(existing, data_type)?;
            changes.push(format!("{existing}: Converted LOD data to {data_type}"));
        }
    } else {
        let added = nif.add_block(data_type, None);
        nif.blocks[lod_id].set_field("LOD Level Data", NifValue::Ref(added as i32));
        data_id = Some(added);
        changes.push(format!("{added}: Added {data_type}"));
    }
    let mut root_refs = value_array(nif.blocks[root_id].get_field("Children")).to_vec();
    root_refs.retain(|value| {
        value_ref(Some(value)).is_none_or(|reference| !shape_ids.contains(&(reference as usize)))
    });
    nif.blocks[root_id].set_field("Num Children", NifValue::UInt(root_refs.len() as u64));
    nif.blocks[root_id].set_field("Children", NifValue::Array(root_refs));
    let mut lod_refs = value_array(nif.blocks[lod_id].get_field("Children")).to_vec();
    for shape_id in shape_ids {
        if !lod_refs
            .iter()
            .any(|value| value_ref(Some(value)) == Some(shape_id as i32))
        {
            lod_refs.push(NifValue::Ref(shape_id as i32));
            changes.push(format!("{shape_id}: Moved shape under NiLODNode"));
        }
    }
    nif.blocks[lod_id].set_field("Num Children", NifValue::UInt(lod_refs.len() as u64));
    nif.blocks[lod_id].set_field("Children", NifValue::Array(lod_refs));
    let child_count = value_array(nif.blocks[lod_id].get_field("Children")).len();
    let data_id = data_id.unwrap();
    if data_type == "NiRangeLODData" {
        let mut extents = vec![0.0];
        extents
            .extend(option_f64_array(options, "extents").unwrap_or_else(|| vec![2000.0, 50000.0]));
        let levels = (0..child_count)
            .map(|index| {
                NifValue::Struct(IndexMap::from([
                    (
                        "Near Extent".to_string(),
                        NifValue::Float(extents.get(index).copied().unwrap_or(0.0)),
                    ),
                    (
                        "Far Extent".to_string(),
                        NifValue::Float(extents.get(index + 1).copied().unwrap_or(0.0)),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let levels_changed = nif.blocks[data_id].get_field("LOD Levels")
            != Some(&NifValue::Array(levels.clone()))
            || value_u64(nif.blocks[data_id].get_field("Num LOD Levels"))
                != Some(levels.len() as u64);
        nif.blocks[data_id].set_field("Num LOD Levels", NifValue::UInt(levels.len() as u64));
        nif.blocks[data_id].set_field("LOD Levels", NifValue::Array(levels));
        if levels_changed {
            changes.push(format!("{data_id}: Updated LOD levels"));
        }
    } else {
        let proportions = option_f64_array(options, "proportions").unwrap_or_else(|| vec![0.48]);
        let levels = proportions
            .into_iter()
            .take(child_count)
            .map(NifValue::Float)
            .collect::<Vec<_>>();
        let levels_changed = nif.blocks[data_id].get_field("Proportion Levels")
            != Some(&NifValue::Array(levels.clone()))
            || value_u64(nif.blocks[data_id].get_field("Num Proportions"))
                != Some(levels.len() as u64);
        nif.blocks[data_id].set_field("Num Proportions", NifValue::UInt(levels.len() as u64));
        nif.blocks[data_id].set_field("Proportion Levels", NifValue::Array(levels));
        if levels_changed {
            changes.push(format!("{data_id}: Updated proportion levels"));
        }
    }
    Ok(changes)
}

fn record_field_change(block_id: usize, field: &str, changed: bool, changes: &mut Vec<String>) {
    if changed {
        changes.push(format!("{block_id} {field}: Updated"));
    }
}

fn set_block_field_if_present(block: &mut NifBlock, name: &str, replacement: NifValue) -> bool {
    if block.get_field(name).is_none() || block.get_field(name) == Some(&replacement) {
        return false;
    }
    block.set_field(name, replacement);
    true
}

fn rigid_body_info_mut(block: &mut NifBlock) -> Option<&mut NifValue> {
    let key = block
        .fields
        .keys()
        .find(|key| bare_name(key) == "Rigid Body Info")
        .cloned()?;
    block.fields.get_mut(&key)
}

fn set_rigid_field_if_present(block: &mut NifBlock, name: &str, replacement: NifValue) -> bool {
    if block.get_field(name).is_some() {
        return set_block_field_if_present(block, name, replacement);
    }
    rigid_body_info_mut(block)
        .is_some_and(|info| set_struct_field_if_present(info, name, replacement))
}

fn set_rigid_nested_field_if_present(
    block: &mut NifBlock,
    group: &str,
    name: &str,
    replacement: NifValue,
) -> bool {
    if let Some(value) = block.get_field_mut(group) {
        return set_struct_field_if_present(value, name, replacement);
    }
    rigid_body_info_mut(block)
        .and_then(|info| nested_value_mut(Some(info), group))
        .is_some_and(|value| set_struct_field_if_present(value, name, replacement))
}

fn set_struct_field_if_present(value: &mut NifValue, name: &str, replacement: NifValue) -> bool {
    let NifValue::Struct(fields) = value else {
        return false;
    };
    let Some(field) = named_value_mut(fields, name) else {
        return false;
    };
    if *field == replacement {
        return false;
    }
    *field = replacement;
    true
}

fn property_by_type(nif: &NifFile, shape_id: usize, type_name: &str) -> Option<usize> {
    value_array(nif.blocks.get(shape_id)?.get_field("Properties"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .find(|reference| {
            nif.get_block(*reference)
                .is_some_and(|block| SCHEMA.is_subtype_of(&block.type_name, type_name))
        })
}

fn set_float_extra_data(nif: &mut NifFile, owner_id: usize, name: &str, value: f64) -> bool {
    let existing = value_array(nif.blocks[owner_id].get_field("Extra Data List"))
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|reference| *reference >= 0)
        .map(|reference| reference as usize)
        .find(|reference| {
            nif.get_block(*reference)
                .filter(|block| block.type_name == "NiFloatExtraData")
                .and_then(|block| block.get_field("Name"))
                .and_then(value_string)
                .is_some_and(|current| current == name)
        });
    let (extra_id, created) = if let Some(extra_id) = existing {
        (extra_id, false)
    } else {
        let extra_id = nif.add_block(
            "NiFloatExtraData",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String(name.to_string())),
                ("Float Data".to_string(), NifValue::Float(value)),
            ])),
        );
        let mut refs = value_array(nif.blocks[owner_id].get_field("Extra Data List")).to_vec();
        refs.push(NifValue::Ref(extra_id as i32));
        nif.blocks[owner_id].set_field("Num Extra Data List", NifValue::UInt(refs.len() as u64));
        nif.blocks[owner_id].set_field("Extra Data List", NifValue::Array(refs));
        (extra_id, true)
    };
    if created {
        return true;
    }
    let old = value_f64(nif.blocks[extra_id].get_field("Float Data"));
    if old.is_some_and(|old| (old - value).abs() <= f64::EPSILON) {
        return false;
    }
    nif.blocks[extra_id].set_field("Float Data", NifValue::Float(value));
    true
}

fn bounding_sphere(positions: &[[f32; 3]]) -> NifValue {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(position[axis]);
            maximum[axis] = maximum[axis].max(position[axis]);
        }
    }
    let center = [
        (minimum[0] + maximum[0]) * 0.5,
        (minimum[1] + maximum[1]) * 0.5,
        (minimum[2] + maximum[2]) * 0.5,
    ];
    let radius = positions
        .iter()
        .map(|position| {
            position
                .iter()
                .zip(center)
                .map(|(value, center)| (value - center).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0f32, f32::max);
    NifValue::Struct(IndexMap::from([
        ("Center".to_string(), NifValue::Vec3(center)),
        ("Radius".to_string(), NifValue::Float(radius as f64)),
    ]))
}

fn replace_assets(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let pairs = replacement_pairs(options)?;
    let case_sensitive = option_bool(options, "case_sensitive").unwrap_or(false);
    let use_regex = option_bool(options, "regex").unwrap_or(false);
    let fix_absolute = option_bool(options, "fix_absolute").unwrap_or(false);
    let patterns = pairs
        .iter()
        .map(|(search, replacement)| {
            if search.is_empty() {
                return Ok((None, replacement.as_str()));
            }
            let source = if use_regex {
                search.clone()
            } else {
                regex::escape(search)
            };
            RegexBuilder::new(&source)
                .case_insensitive(!case_sensitive)
                .build()
                .map(|pattern| (Some(pattern), replacement.as_str()))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let modern_material_names = matches!(
        crate::validation::nif_game_label(nif),
        "fo4" | "fo76" | "starfield"
    );
    let mut changes = Vec::new();
    for block in &mut nif.blocks {
        let block_id = block.block_id;
        let block_type = block.type_name.clone();
        match block_type.as_str() {
            "BSShaderTextureSet" => {
                if let Some(NifValue::Array(textures)) = block.get_field_mut("Textures") {
                    for (index, texture) in textures.iter_mut().enumerate() {
                        replace_asset_value(
                            texture,
                            &patterns,
                            use_regex,
                            fix_absolute,
                            block_id,
                            &format!("Textures[{index}]"),
                            &mut changes,
                        );
                    }
                }
            }
            "BSLightingShaderProperty" => {
                if modern_material_names
                    && block
                        .get_field("Name")
                        .and_then(value_string)
                        .is_some_and(is_material_path)
                {
                    if let Some(value) = block.get_field_mut("Name") {
                        replace_asset_value(
                            value,
                            &patterns,
                            use_regex,
                            fix_absolute,
                            block_id,
                            "Name",
                            &mut changes,
                        );
                    }
                }
            }
            "BSEffectShaderProperty" => {
                for field in [
                    "Source Texture",
                    "Grayscale Texture",
                    "Greyscale Texture",
                    "Env Map Texture",
                    "Normal Texture",
                    "Env Mask Texture",
                ] {
                    if let Some(value) = block.get_field_mut(field) {
                        replace_asset_value(
                            value,
                            &patterns,
                            use_regex,
                            fix_absolute,
                            block_id,
                            field,
                            &mut changes,
                        );
                    }
                }
                if modern_material_names
                    && block
                        .get_field("Name")
                        .and_then(value_string)
                        .is_some_and(is_material_path)
                {
                    if let Some(value) = block.get_field_mut("Name") {
                        replace_asset_value(
                            value,
                            &patterns,
                            use_regex,
                            fix_absolute,
                            block_id,
                            "Name",
                            &mut changes,
                        );
                    }
                }
            }
            "BSShaderNoLightingProperty" | "TallGrassShaderProperty" | "TileShaderProperty" => {
                if let Some(value) = block.get_field_mut("File Name") {
                    replace_asset_value(
                        value,
                        &patterns,
                        use_regex,
                        fix_absolute,
                        block_id,
                        "File Name",
                        &mut changes,
                    );
                }
            }
            "BSSkyShaderProperty" => {
                if let Some(value) = block.get_field_mut("Source Texture") {
                    replace_asset_value(
                        value,
                        &patterns,
                        use_regex,
                        fix_absolute,
                        block_id,
                        "Source Texture",
                        &mut changes,
                    );
                }
            }
            "BSBehaviorGraphExtraData" => {
                if let Some(value) = block.get_field_mut("Behavior Graph File") {
                    replace_asset_value(
                        value,
                        &patterns,
                        use_regex,
                        fix_absolute,
                        block_id,
                        "Behavior Graph File",
                        &mut changes,
                    );
                }
            }
            "BSSubIndexTriShape" => {
                if let Some(value) =
                    nested_value_mut(block.get_field_mut("Segment Data"), "SSF File")
                {
                    replace_asset_value(
                        value,
                        &patterns,
                        use_regex,
                        fix_absolute,
                        block_id,
                        "Segment Data.SSF File",
                        &mut changes,
                    );
                }
            }
            _ if SCHEMA.is_subtype_of(&block_type, "NiTexture") => {
                if let Some(value) = block.get_field_mut("File Name") {
                    replace_asset_value(
                        value,
                        &patterns,
                        use_regex,
                        fix_absolute,
                        block_id,
                        "File Name",
                        &mut changes,
                    );
                }
            }
            _ => {}
        }
    }
    Ok(changes)
}

fn replacement_pairs(options: &Value) -> Result<Vec<(String, String)>, String> {
    if let Some(pairs) = options.get("pairs").and_then(Value::as_array) {
        let pairs = pairs
            .iter()
            .enumerate()
            .map(|(index, pair)| {
                let pair = pair
                    .as_array()
                    .ok_or_else(|| format!("pairs[{index}] must be a two-item array"))?;
                if pair.len() != 2 {
                    return Err(format!(
                        "pairs[{index}] must contain search and replacement"
                    ));
                }
                let search = pair[0]
                    .as_str()
                    .ok_or_else(|| format!("pairs[{index}][0] must be a string"))?;
                let replacement = pair[1]
                    .as_str()
                    .ok_or_else(|| format!("pairs[{index}][1] must be a string"))?;
                Ok((search.to_string(), replacement.to_string()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if !pairs.is_empty() {
            return Ok(pairs);
        }
    }
    let search = options
        .get("search")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let replacement = options
        .get("replace")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if search.is_empty() && replacement.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![(search.to_string(), replacement.to_string())])
    }
}

fn replace_asset_value(
    value: &mut NifValue,
    patterns: &[(Option<Regex>, &str)],
    use_regex: bool,
    fix_absolute: bool,
    block_id: usize,
    path: &str,
    changes: &mut Vec<String>,
) {
    let Some(text) = value_string_mut(value) else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let original = text.clone();
    let mut replaced = text.trim().to_string();
    for (pattern, replacement) in patterns {
        if let Some(pattern) = pattern {
            replaced = if use_regex {
                pattern.replace_all(&replaced, *replacement).into_owned()
            } else {
                pattern
                    .replace_all(&replaced, NoExpand(*replacement))
                    .into_owned()
            };
        } else {
            replaced = format!("{replacement}{replaced}");
        }
    }
    if fix_absolute {
        replaced = truncate_absolute_asset_path(&replaced);
    }
    if original != replaced {
        changes.push(format!(
            "{block_id} {path}: Replaced {original:?} with {replaced:?}"
        ));
        *text = replaced;
    }
}

fn truncate_absolute_asset_path(path: &str) -> String {
    if path.as_bytes().get(1) != Some(&b':') {
        return path.to_string();
    }
    let lower = path.to_ascii_lowercase().replace('/', "\\");
    for marker in ["\\data\\", "\\data files\\"] {
        if let Some(index) = lower.find(marker) {
            return path[index + marker.len()..].to_string();
        }
    }
    path.to_string()
}

fn is_material_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".bgsm") || lower.ends_with(".bgem")
}

fn remove_unused_nodes(nif: &mut NifFile, options: &Value) -> Vec<String> {
    if option_bool(options, "single_root").unwrap_or(true) && nif.header.footer_roots.len() > 1 {
        nif.header.footer_roots.truncate(1);
    }
    let roots = roots(nif);
    let mut reachable = HashSet::new();
    let mut stack = roots;
    while let Some(block_id) = stack.pop() {
        if !reachable.insert(block_id) {
            continue;
        }
        if let Some(block) = nif.get_block(block_id) {
            stack.extend(
                block
                    .get_refs(&SCHEMA)
                    .into_iter()
                    .filter(|reference| *reference >= 0)
                    .map(|reference| reference as usize),
            );
        }
    }
    let removed = nif
        .blocks
        .iter()
        .filter(|block| !reachable.contains(&block.block_id))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    if removed.is_empty() {
        return Vec::new();
    }
    nif.remove_blocks(&removed);
    vec![format!("Removed {} unused block(s)", removed.len())]
}

fn convert_block_types(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let from = required_string(options, "from")?;
    let to = required_string(options, "to")?;
    let root_only = option_bool(options, "root_only").unwrap_or(false);
    let roots = roots(nif).into_iter().collect::<HashSet<_>>();
    let ids = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == from && (!root_only || roots.contains(&block.block_id)))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    for block_id in ids {
        if nif.convert_block_type(block_id, to)? {
            apply_conversion_defaults(nif, block_id);
            changes.push(format!("{block_id}: Converted {from} to {to}"));
        }
    }
    Ok(changes)
}

fn apply_conversion_defaults(nif: &mut NifFile, block_id: usize) {
    let block_type = nif.blocks[block_id].type_name.clone();
    if block_type == "bhkListShape" {
        nif.blocks[block_id].set_field(
            "Unknown Ints",
            NifValue::Array(vec![NifValue::UInt(0), NifValue::UInt(0)]),
        );
    } else if block_type == "Lighting30ShaderProperty" {
        nif.blocks[block_id].set_field("Shader Type", NifValue::UInt(29));
    } else if block_type == "BSDismemberSkinInstance" {
        let count = referenced_block(nif, nif.blocks[block_id].get_field("Skin Partition"))
            .and_then(|partition| value_u64(partition.get_field("Num Partitions")))
            .unwrap_or(1);
        let block = &mut nif.blocks[block_id];
        block.set_field("Num Partitions", NifValue::UInt(count));
        block.set_field(
            "Partitions",
            NifValue::Array((0..count).map(|_| NifValue::UInt(0)).collect()),
        );
    }
}

fn set_missing_names(nif: &mut NifFile, input: &Path, options: &Value) -> Vec<String> {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mesh");
    let rename_root = option_bool(options, "rename_root").unwrap_or(true);
    let root_ids = roots(nif).into_iter().collect::<HashSet<_>>();
    let mut used = nif
        .blocks
        .iter()
        .filter_map(|block| block.get_field("Name").and_then(value_string))
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut index = 0usize;
    let mut changes = Vec::new();
    for block in &mut nif.blocks {
        if !SCHEMA.is_subtype_of(&block.type_name, "NiAVObject") {
            continue;
        }
        let current = block
            .get_field("Name")
            .and_then(value_string)
            .unwrap_or_default();
        if rename_root && root_ids.contains(&block.block_id) {
            if !current.eq_ignore_ascii_case(stem) {
                let name = unique_name(stem, &mut used);
                block.set_field("Name", NifValue::String(name.clone()));
                changes.push(format!("{}: Set root name to {name:?}", block.block_id));
            }
        } else if current.is_empty() {
            let name = loop {
                let candidate = format!("{stem}:{index}");
                index += 1;
                if used.insert(candidate.to_ascii_lowercase()) {
                    break candidate;
                }
            };
            block.set_field("Name", NifValue::String(name.clone()));
            changes.push(format!("{}: Set missing name to {name:?}", block.block_id));
        }
    }
    changes
}

fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_ascii_lowercase()) {
        return base.to_string();
    }
    let mut suffix = 0usize;
    loop {
        let candidate = format!("{base}:{suffix}");
        suffix += 1;
        if used.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
}

fn unskin_mesh(nif: &mut NifFile) -> Vec<String> {
    let flags1_field = if crate::validation::nif_game_label(nif) == "fo3/fnv" {
        "Shader Flags"
    } else {
        "Shader Flags 1"
    };
    let bones = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiNode"))
        .filter(|block| block_is_bone(nif, block.block_id))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changes = if bones.is_empty() {
        Vec::new()
    } else {
        let count = bones.len();
        nif.remove_blocks(&bones);
        vec![format!("Removed {count} bone node(s)")]
    };
    let shapes = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom"))
        .filter_map(|block| {
            value_ref(block.get_field("Skin Instance"))
                .filter(|skin| *skin >= 0)
                .map(|_| block.block_id)
        })
        .collect::<Vec<_>>();
    for shape_id in shapes {
        nif.blocks[shape_id].set_field("Skin Instance", NifValue::Ref(-1));
        if let Some(shader_id) = geometry_shader_id(nif, shape_id) {
            let flags = value_u64(nif.blocks[shader_id].get_field(flags1_field)).unwrap_or(0);
            nif.blocks[shader_id].set_field(flags1_field, NifValue::UInt(flags & !(1 << 1)));
        }
        changes.push(format!("{shape_id}: Removed skin instance"));
    }
    changes.extend(remove_unused_nodes(nif, &json!({"single_root": false})));
    changes.extend(update_bounds(nif));
    changes
}

fn geometry_shader_id(nif: &NifFile, shape_id: usize) -> Option<usize> {
    let shape = nif.get_block(shape_id)?;
    referenced_block(nif, shape.get_field("Shader Property"))
        .map(|shader| shader.block_id)
        .or_else(|| {
            value_array(shape.get_field("Properties"))
                .iter()
                .filter_map(|value| referenced_block(nif, Some(value)))
                .find(|block| SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty"))
                .map(|shader| shader.block_id)
        })
}

fn roots(nif: &NifFile) -> Vec<usize> {
    let roots = nif
        .header
        .footer_roots
        .iter()
        .filter(|root| **root >= 0)
        .map(|root| *root as usize)
        .filter(|root| *root < nif.blocks.len())
        .collect::<Vec<_>>();
    if roots.is_empty() && !nif.blocks.is_empty() {
        vec![0]
    } else {
        roots
    }
}

fn positions(block: &NifBlock) -> Vec<[f32; 3]> {
    let values = if !value_array(block.get_field("Vertex Data")).is_empty() {
        value_array(block.get_field("Vertex Data"))
    } else {
        value_array(block.get_field("Vertices"))
    };
    values
        .iter()
        .filter_map(|value| {
            let value = nested_value(Some(value), "Vertex").unwrap_or(value);
            match value {
                NifValue::Vec3(position) => Some(*position),
                NifValue::Struct(fields) => Some([
                    named_value(fields, "x").and_then(|value| value_f64(Some(value)))? as f32,
                    named_value(fields, "y").and_then(|value| value_f64(Some(value)))? as f32,
                    named_value(fields, "z").and_then(|value| value_f64(Some(value)))? as f32,
                ]),
                _ => None,
            }
        })
        .collect()
}

fn referenced_block<'a>(nif: &'a NifFile, value: Option<&NifValue>) -> Option<&'a NifBlock> {
    value_ref(value)
        .filter(|reference| *reference >= 0)
        .and_then(|reference| nif.get_block(reference as usize))
}

fn named_value<'a>(fields: &'a IndexMap<String, NifValue>, name: &str) -> Option<&'a NifValue> {
    fields.get(name).or_else(|| {
        fields
            .iter()
            .find(|(field, _)| bare_name(field).eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

fn named_value_mut<'a>(
    fields: &'a mut IndexMap<String, NifValue>,
    name: &str,
) -> Option<&'a mut NifValue> {
    if fields.contains_key(name) {
        return fields.get_mut(name);
    }
    let key = fields
        .keys()
        .find(|field| bare_name(field).eq_ignore_ascii_case(name))
        .cloned()?;
    fields.get_mut(&key)
}

fn nested_value<'a>(value: Option<&'a NifValue>, name: &str) -> Option<&'a NifValue> {
    let NifValue::Struct(fields) = value? else {
        return None;
    };
    named_value(fields, name)
}

fn nested_value_mut<'a>(value: Option<&'a mut NifValue>, name: &str) -> Option<&'a mut NifValue> {
    let NifValue::Struct(fields) = value? else {
        return None;
    };
    if fields.contains_key(name) {
        return fields.get_mut(name);
    }
    let key = fields
        .keys()
        .find(|field| bare_name(field).eq_ignore_ascii_case(name))
        .cloned()?;
    fields.get_mut(&key)
}

fn nested_array<'a>(value: Option<&'a NifValue>, name: &str) -> &'a [NifValue] {
    value_array(nested_value(value, name))
}

fn nested_u64(value: Option<&NifValue>, name: &str) -> Option<u64> {
    value_u64(nested_value(value, name))
}

fn value_array(value: Option<&NifValue>) -> &[NifValue] {
    match value {
        Some(NifValue::Array(values)) => values,
        _ => &[],
    }
}

fn value_ref(value: Option<&NifValue>) -> Option<i32> {
    match value? {
        NifValue::Ref(value) => Some(*value),
        NifValue::Int(value) => i32::try_from(*value).ok(),
        NifValue::UInt(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

fn value_string(value: &NifValue) -> Option<&str> {
    match value {
        NifValue::String(value) | NifValue::Char(value) => Some(value),
        _ => None,
    }
}

fn value_string_mut(value: &mut NifValue) -> Option<&mut String> {
    match value {
        NifValue::String(value) | NifValue::Char(value) => Some(value),
        _ => None,
    }
}

fn vec3_value(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(value) | NifValue::Color3(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            named_value(fields, "x").and_then(|value| value_f64(Some(value)))? as f32,
            named_value(fields, "y").and_then(|value| value_f64(Some(value)))? as f32,
            named_value(fields, "z").and_then(|value| value_f64(Some(value)))? as f32,
        ]),
        _ => None,
    }
}

fn vec4_xyz(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec4(value) | NifValue::Quaternion(value) => Some([value[0], value[1], value[2]]),
        NifValue::Struct(fields) => Some([
            named_value(fields, "x").and_then(|value| value_f64(Some(value)))? as f32,
            named_value(fields, "y").and_then(|value| value_f64(Some(value)))? as f32,
            named_value(fields, "z").and_then(|value| value_f64(Some(value)))? as f32,
        ]),
        _ => None,
    }
}

fn set_vec4_xyz(value: &mut NifValue, xyz: [f32; 3]) {
    match value {
        NifValue::Vec4(value) | NifValue::Quaternion(value) => {
            value[..3].copy_from_slice(&xyz);
        }
        NifValue::Struct(fields) => {
            for (name, component) in ["x", "y", "z"].into_iter().zip(xyz) {
                if let Some(value) = named_value_mut(fields, name) {
                    *value = NifValue::Float(component as f64);
                }
            }
        }
        _ => {}
    }
}

fn uv_value(value: &NifValue) -> Option<[f32; 2]> {
    let NifValue::Struct(fields) = value else {
        return None;
    };
    Some([
        named_value(fields, "u").and_then(|value| value_f64(Some(value)))? as f32,
        named_value(fields, "v").and_then(|value| value_f64(Some(value)))? as f32,
    ])
}

fn triangle_value(value: &NifValue) -> Option<[u32; 3]> {
    let values = match value {
        NifValue::Array(values) if values.len() == 3 => values.iter().collect::<Vec<_>>(),
        NifValue::Struct(fields) => ["v1", "v2", "v3"]
            .iter()
            .map(|name| named_value(fields, name))
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    Some([
        u32::try_from(value_u64(Some(values[0]))?).ok()?,
        u32::try_from(value_u64(Some(values[1]))?).ok()?,
        u32::try_from(value_u64(Some(values[2]))?).ok()?,
    ])
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

fn value_u64(value: Option<&NifValue>) -> Option<u64> {
    match value? {
        NifValue::UInt(value) => Some(*value),
        NifValue::Int(value) => u64::try_from(*value).ok(),
        _ => None,
    }
}

fn bare_name(name: &str) -> &str {
    name.split_once(':').map(|(name, _)| name).unwrap_or(name)
}

fn required_string<'a>(options: &'a Value, name: &str) -> Result<&'a str, String> {
    options
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing string option: {name}"))
}

fn option_bool(options: &Value, name: &str) -> Option<bool> {
    options.get(name).and_then(Value::as_bool)
}

fn option_u64(options: &Value, name: &str) -> Option<u64> {
    json_u64(options.get(name))
}

fn option_f64(options: &Value, name: &str) -> Option<f64> {
    json_f64(options.get(name))
}

fn option_vec3(options: &Value, name: &str) -> Option<[f32; 3]> {
    let values = options.get(name)?.as_array()?;
    if values.len() != 3 {
        return None;
    }
    Some([
        json_f64(values.first())? as f32,
        json_f64(values.get(1))? as f32,
        json_f64(values.get(2))? as f32,
    ])
}

fn option_f64_array(options: &Value, name: &str) -> Option<Vec<f64>> {
    options
        .get(name)?
        .as_array()?
        .iter()
        .map(|value| json_f64(Some(value)))
        .collect()
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value.as_u64().or_else(|| {
            value.as_str().and_then(|value| {
                value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                    .map_or_else(
                        || value.parse().ok(),
                        |hex| u64::from_str_radix(hex, 16).ok(),
                    )
            })
        })
    })
}

fn setting_u64(value: Option<&Value>, enum_types: &[&str]) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some(value) = json_u64(Some(value)) {
        return Ok(Some(value));
    }
    let text = value
        .as_str()
        .ok_or_else(|| format!("Invalid enum setting: {value}"))?;
    for type_name in enum_types {
        if let Some(value) = SCHEMA.get_enum(type_name).and_then(|definition| {
            definition
                .options
                .iter()
                .find(|option| option.name.eq_ignore_ascii_case(text))
                .map(|option| option.value)
        }) {
            return u64::try_from(value)
                .map(Some)
                .map_err(|_| format!("{type_name} value {text:?} is negative"));
        }
    }
    Err(format!("Unknown {} value {text:?}", enum_types.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation_sequence(entries: Vec<NifValue>) -> NifFile {
        let mut nif = NifFile::new("oblivion");
        nif.convert_block_type(0, "NiControllerSequence").unwrap();
        nif.blocks[0].set_field(
            "Num Controlled Blocks",
            NifValue::UInt(entries.len() as u64),
        );
        nif.blocks[0].set_field("Controlled Blocks", NifValue::Array(entries));
        nif
    }

    fn controlled_entry(
        name: &str,
        controller_type: &str,
        priority: u64,
        interpolator: i32,
    ) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("Node Name".to_string(), NifValue::String(name.to_string())),
            (
                "Controller Type".to_string(),
                NifValue::String(controller_type.to_string()),
            ),
            ("Priority".to_string(), NifValue::UInt(priority)),
            ("Interpolator".to_string(), NifValue::Ref(interpolator)),
        ]))
    }

    #[test]
    fn oblivion_tangents_use_binary_data_bytearray() {
        let mut nif = NifFile::new("oblivion");
        let shape = nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([(
                "Extra Data List".to_string(),
                NifValue::Array(Vec::new()),
            )])),
        );

        write_oblivion_tangents(&mut nif, shape, &[[1.0, 0.0, 0.0]], &[[0.0, 1.0, 0.0]]);

        let extra = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiBinaryExtraData")
            .unwrap();
        let binary = extra.get_field("Binary Data").unwrap();
        assert_eq!(nested_u64(Some(binary), "Data Size"), Some(24));
        assert!(matches!(
            nested_value(Some(binary), "Data"),
            Some(NifValue::Bytes(bytes)) if bytes.len() == 24
        ));
        assert!(extra.get_field("Data").is_none());
    }

    #[test]
    fn remove_unused_nodes_preserves_and_remaps_footer_root() {
        let mut nif = NifFile::new("oblivion");
        nif.header.footer_roots = vec![1];
        nif.add_block("NiNode", None);
        let changes = remove_unused_nodes(&mut nif, &json!({}));
        assert_eq!(nif.blocks.len(), 1);
        assert_eq!(nif.header.footer_roots, vec![0]);
        assert_eq!(changes, vec!["Removed 1 unused block(s)"]);
    }

    #[test]
    fn set_missing_names_uses_input_stem() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::String(String::new()));
        let changes = set_missing_names(
            &mut nif,
            Path::new("meshes/example.nif"),
            &json!({"rename_root": true}),
        );
        assert_eq!(
            value_string(nif.blocks[0].get_field("Name").unwrap()),
            Some("example")
        );
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn replace_assets_only_changes_registered_asset_fields() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::String("textures\\old\\root".to_string()));
        let texture_id = nif.add_block(
            "BSShaderTextureSet",
            Some(IndexMap::from([(
                "Textures".to_string(),
                NifValue::Array(vec![NifValue::String(
                    "C:\\Game\\Data\\textures\\old\\a.dds".to_string(),
                )]),
            )])),
        );
        let changes = replace_assets(
            &mut nif,
            &json!({
                "pairs": [["textures\\old", "textures\\new"]],
                "fix_absolute": true,
            }),
        )
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            value_array(nif.blocks[texture_id].get_field("Textures"))[0],
            NifValue::String("textures\\new\\a.dds".to_string())
        );
        assert_eq!(
            value_string(nif.blocks[0].get_field("Name").unwrap()),
            Some("textures\\old\\root")
        );
    }

    #[test]
    fn replace_assets_uses_literal_replacement_without_regex_mode() {
        let mut nif = NifFile::new("fo4");
        let texture_id = nif.add_block(
            "BSShaderTextureSet",
            Some(IndexMap::from([(
                "Textures".to_string(),
                NifValue::Array(vec![NifValue::String("old.dds".to_string())]),
            )])),
        );
        replace_assets(&mut nif, &json!({"pairs": [["old", "$1"]], "regex": false})).unwrap();
        assert_eq!(
            value_array(nif.blocks[texture_id].get_field("Textures"))[0],
            NifValue::String("$1.dds".to_string())
        );
    }

    #[test]
    fn update_tangents_adds_modern_vertex_attribute() {
        let mut nif = NifFile::new("fo4");
        let vertex = |position: [f32; 3], uv: [f64; 2]| {
            NifValue::Struct(IndexMap::from([
                ("Vertex".to_string(), NifValue::Vec3(position)),
                (
                    "UV".to_string(),
                    NifValue::Struct(IndexMap::from([
                        ("u".to_string(), NifValue::Float(uv[0])),
                        ("v".to_string(), NifValue::Float(uv[1])),
                    ])),
                ),
                ("Normal".to_string(), NifValue::Vec3([0.0, 0.0, 1.0])),
            ]))
        };
        let triangle = NifValue::Struct(IndexMap::from([
            ("v1".to_string(), NifValue::UInt(0)),
            ("v2".to_string(), NifValue::UInt(1)),
            ("v3".to_string(), NifValue::UInt(2)),
        ]));
        let shape_id = nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                (
                    "Vertex Desc".to_string(),
                    NifValue::UInt(4 | (2 << 8) | (3 << 16) | (0xbu64 << 44)),
                ),
                (
                    "Vertex Data".to_string(),
                    NifValue::Array(vec![
                        vertex([0.0, 0.0, 0.0], [0.0, 0.0]),
                        vertex([1.0, 0.0, 0.0], [1.0, 0.0]),
                        vertex([0.0, 1.0, 0.0], [0.0, 1.0]),
                    ]),
                ),
                ("Triangles".to_string(), NifValue::Array(vec![triangle])),
            ])),
        );
        let changes = update_tangents(&mut nif, &json!({"add_if_missing": true}));
        assert_eq!(changes.len(), 1);
        assert_ne!(vertex_desc_flags(&nif.blocks[shape_id]) & VF_TANGENTS, 0);
        assert!(
            nested_value(
                value_array(nif.blocks[shape_id].get_field("Vertex Data")).first(),
                "Tangent",
            )
            .is_some()
        );
    }

    #[test]
    fn convert_block_type_preserves_root_reference() {
        let mut nif = NifFile::new("fo4");
        let changes = convert_block_types(
            &mut nif,
            &json!({"from": "BSFadeNode", "to": "NiNode", "root_only": true}),
        )
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(nif.blocks[0].type_name, "NiNode");
        assert_eq!(nif.header.footer_roots, vec![0]);
    }

    #[test]
    fn shader_flag_modes_match_nif() {
        let mut nif = NifFile::new("fo4");
        let shader_id = nif.add_block(
            "BSLightingShaderProperty",
            Some(IndexMap::from([
                ("Shader Flags 1".to_string(), NifValue::UInt(0b0011)),
                ("Shader Flags 2".to_string(), NifValue::UInt(0b1100)),
            ])),
        );
        update_shader_flags(
            &mut nif,
            &json!({"flags1": 0b0100, "flags2": 0b1000, "mode": "add"}),
        )
        .unwrap();
        assert_eq!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 1")),
            Some(0b0111)
        );
        update_shader_flags(
            &mut nif,
            &json!({"flags1": 0b0011, "flags2": 0b0100, "mode": "remove"}),
        )
        .unwrap();
        assert_eq!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 1")),
            Some(0b0100)
        );
        assert_eq!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 2")),
            Some(0b1000)
        );
    }

    #[test]
    fn fo3_shader_actions_use_the_fo3_shader_flags_field() {
        let mut nif = NifFile::new("fnv");
        let shader_id = nif.add_block(
            "BSShaderPPLightingProperty",
            Some(IndexMap::from([
                ("Shader Flags".to_string(), NifValue::UInt(1 << 7)),
                ("Shader Flags 2".to_string(), NifValue::UInt(0)),
            ])),
        );
        let _shape_id = nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([(
                "Properties".to_string(),
                NifValue::Array(vec![NifValue::Ref(shader_id as i32)]),
            )])),
        );

        update_shader_flags(&mut nif, &json!({"flags1": 2, "mode": "add"})).unwrap();
        walls_reflection_flag(&mut nif, &json!({}));

        assert_eq!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags")),
            Some((1 << 7) | 2)
        );
        assert_ne!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 2")).unwrap() & (1 << 31),
            0
        );
    }

    #[test]
    fn nvse_soft_particles_sets_flag_and_scale_extra_data() {
        let mut nif = NifFile::new("fnv");
        let shader_id = nif.add_block(
            "BSShaderNoLightingProperty",
            Some(IndexMap::from([(
                "Shader Flags 2".to_string(),
                NifValue::UInt(0),
            )])),
        );
        let shape_id = nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([
                (
                    "Properties".to_string(),
                    NifValue::Array(vec![NifValue::Ref(shader_id as i32)]),
                ),
                ("Extra Data List".to_string(), NifValue::Array(Vec::new())),
            ])),
        );
        let changes = soft_particles(&mut nif, &json!({"soft_scale": 0.05}));
        assert_eq!(changes.len(), 2);
        assert_ne!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 2")).unwrap() & (1 << 30),
            0
        );
        let extra_id =
            value_ref(value_array(nif.blocks[shape_id].get_field("Extra Data List")).first())
                .unwrap() as usize;
        assert_eq!(
            value_string(nif.blocks[extra_id].get_field("Name").unwrap()),
            Some("VPSoftScale")
        );
    }

    #[test]
    fn ragdoll_motor_is_cross_product_and_can_be_wrapped() {
        let mut nif = NifFile::new("skyrimse");
        let ragdoll = NifValue::Struct(IndexMap::from([
            ("Twist A".to_string(), NifValue::Vec4([1.0, 0.0, 0.0, 0.0])),
            ("Plane A".to_string(), NifValue::Vec4([0.0, 1.0, 0.0, 0.0])),
            ("Motor A".to_string(), NifValue::Vec4([0.0, 0.0, 0.0, 0.0])),
            ("Twist B".to_string(), NifValue::Vec4([0.0, 1.0, 0.0, 0.0])),
            ("Plane B".to_string(), NifValue::Vec4([0.0, 0.0, 1.0, 0.0])),
            ("Motor B".to_string(), NifValue::Vec4([0.0, 0.0, 0.0, 0.0])),
        ]));
        let constraint_id = nif.add_block(
            "bhkRagdollConstraint",
            Some(IndexMap::from([("Constraint".to_string(), ragdoll)])),
        );
        let changes =
            update_ragdoll_constraints(&mut nif, &json!({"convert_to_malleable": true})).unwrap();
        assert_eq!(
            nif.blocks[constraint_id].type_name,
            "bhkMalleableConstraint"
        );
        let malleable = nif.blocks[constraint_id].get_field("Constraint");
        assert_eq!(nested_u64(malleable, "Type"), Some(7));
        let ragdoll = nested_value(malleable, "Ragdoll").unwrap();
        assert_eq!(
            nested_value(Some(ragdoll), "Motor A").and_then(vec4_xyz),
            Some([0.0, 0.0, 1.0])
        );
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn havok_settings_update_nested_rigid_body_info() {
        let mut nif = NifFile::new("fnv");
        let body_id = nif.add_block(
            "bhkRigidBody",
            Some(IndexMap::from([(
                "Rigid Body Info:550_660".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("Mass".to_string(), NifValue::Float(2.0)),
                    ("Linear Damping".to_string(), NifValue::Float(0.1)),
                    (
                        "Havok Filter".to_string(),
                        NifValue::Struct(IndexMap::from([(
                            "Layer".to_string(),
                            NifValue::UInt(1),
                        )])),
                    ),
                    (
                        "Inertia Tensor".to_string(),
                        NifValue::Matrix33([[1.0; 3]; 3]),
                    ),
                ])),
            )])),
        );
        let changes = update_havok_settings(
            &mut nif,
            &json!({"settings": {"mass": 0.0, "linear_damping": 0.25, "layer": "FOL_BIPED"}}),
        )
        .unwrap();
        let info = nif.blocks[body_id].get_field("Rigid Body Info");
        assert_eq!(
            nested_value(info, "Mass").and_then(|value| value_f64(Some(value))),
            Some(0.0)
        );
        assert_eq!(
            nested_u64(nif.blocks[body_id].get_field("Havok Filter"), "Layer"),
            Some(8)
        );
        assert_eq!(
            nested_value(info, "Inertia Tensor"),
            Some(&NifValue::Matrix33([[0.0; 3]; 3]))
        );
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn havok_inertia_uses_shape_dimensions_and_body_part_multiplier() {
        let mut nif = NifFile::new("fnv");
        let shape_id = nif.add_block(
            "bhkBoxShape",
            Some(IndexMap::from([(
                "Dimensions".to_string(),
                NifValue::Vec3([2.0, 4.0, 6.0]),
            )])),
        );
        let body_id = nif.add_block(
            "bhkRigidBody",
            Some(IndexMap::from([
                ("Shape".to_string(), NifValue::Ref(shape_id as i32)),
                (
                    "Rigid Body Info:550_660".to_string(),
                    NifValue::Struct(IndexMap::from([
                        ("Mass".to_string(), NifValue::Float(12.0)),
                        ("Motion System".to_string(), NifValue::UInt(1)),
                        (
                            "Havok Filter".to_string(),
                            NifValue::Struct(IndexMap::from([
                                ("Layer".to_string(), NifValue::UInt(1)),
                                ("Flags and Part Number".to_string(), NifValue::UInt(1)),
                            ])),
                        ),
                        (
                            "Inertia Tensor".to_string(),
                            NifValue::Matrix33([[0.0; 3]; 3]),
                        ),
                        ("Center".to_string(), NifValue::Vec4([0.0; 4])),
                        ("Penetration Depth".to_string(), NifValue::Float(0.0)),
                    ])),
                ),
            ])),
        );
        let changes = update_havok_inertia(&mut nif, &json!({})).unwrap();
        assert_eq!(changes.len(), 1);
        let info = nif.blocks[body_id].get_field("Rigid Body Info");
        assert_eq!(
            nested_value(info, "Inertia Tensor"),
            Some(&NifValue::Matrix33([
                [104.0, 0.0, 0.0],
                [0.0, 80.0, 0.0],
                [0.0, 0.0, 40.0]
            ]))
        );
    }

    #[test]
    fn fo3_packed_mopp_penetration_uses_fo3_game_units() {
        let mut nif = NifFile::new("fnv");
        let data = nif.add_block(
            "hkPackedNiTriStripsData",
            Some(IndexMap::from([(
                "Vertices".to_string(),
                NifValue::Array(vec![
                    NifValue::Vec3([0.0, 0.0, 0.0]),
                    NifValue::Vec3([2.0, 0.0, 0.0]),
                ]),
            )])),
        );
        let packed = nif.add_block(
            "bhkPackedNiTriStripsShape",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data as i32),
            )])),
        );
        let mopp = nif.add_block(
            "bhkMoppBvTreeShape",
            Some(IndexMap::from([(
                "Shape".to_string(),
                NifValue::Ref(packed as i32),
            )])),
        );
        let body = nif.add_block(
            "bhkRigidBody",
            Some(IndexMap::from([
                ("Shape".to_string(), NifValue::Ref(mopp as i32)),
                (
                    "Rigid Body Info:550_660".to_string(),
                    NifValue::Struct(IndexMap::from([
                        ("Mass".to_string(), NifValue::Float(1.0)),
                        ("Motion System".to_string(), NifValue::UInt(1)),
                        (
                            "Havok Filter".to_string(),
                            NifValue::Struct(IndexMap::from([(
                                "Layer".to_string(),
                                NifValue::UInt(1),
                            )])),
                        ),
                        ("Penetration Depth".to_string(), NifValue::Float(0.0)),
                    ])),
                ),
            ])),
        );
        update_havok_inertia(
            &mut nif,
            &json!({
                "update_inertia": false,
                "update_center": false,
                "update_penetration": true
            }),
        )
        .unwrap();
        let depth = rigid_value(&nif.blocks[body], "Penetration Depth")
            .and_then(|value| value_f64(Some(value)))
            .unwrap();
        assert!((depth - 0.4 / 6.999125).abs() < 1.0e-8);
    }

    #[test]
    fn havok_material_search_reports_or_replaces() {
        let mut nif = NifFile::new("skyrimse");
        let shape_id = nif.add_block(
            "bhkBoxShape",
            Some(IndexMap::from([(
                "Material".to_string(),
                NifValue::UInt(493_553_910),
            )])),
        );
        let reported = search_havok_material(
            &mut nif,
            &json!({"material_search": 493_553_910, "report_only": true}),
        )
        .unwrap();
        assert_eq!(reported, vec![format!("{shape_id}: Material 493553910")]);
        search_havok_material(
            &mut nif,
            &json!({
                "material_search": "SKY_HAV_MAT_BOTTLE",
                "material_replace": "SKY_HAV_MAT_BONE_ACTOR"
            }),
        )
        .unwrap();
        assert_eq!(
            value_u64(nif.blocks[shape_id].get_field("Material")),
            Some(2_058_949_504)
        );
    }

    #[test]
    fn animation_processors_edit_controlled_blocks_and_keys() {
        let mut nif = NifFile::new("fnv");
        let key = |time: f64| {
            NifValue::Struct(IndexMap::from([
                ("Time".to_string(), NifValue::Float(time)),
                ("Value".to_string(), NifValue::Vec3([1.0, 2.0, 3.0])),
            ]))
        };
        let data_id = nif.add_block(
            "NiTransformData",
            Some(IndexMap::from([(
                "Translations".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("Interpolation".to_string(), NifValue::UInt(2)),
                    ("Num Keys".to_string(), NifValue::UInt(4)),
                    (
                        "Keys".to_string(),
                        NifValue::Array(vec![key(0.0), key(1.0), key(2.0), key(3.0)]),
                    ),
                ])),
            )])),
        );
        let interpolator_id = nif.add_block(
            "NiTransformInterpolator",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data_id as i32),
            )])),
        );
        nif.blocks[0].type_name = "NiControllerSequence".to_string();
        nif.blocks[0].set_field(
            "Controlled Blocks",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                (
                    "Node Name".to_string(),
                    NifValue::String("Head".to_string()),
                ),
                (
                    "Controller Type".to_string(),
                    NifValue::String(String::new()),
                ),
                (
                    "Interpolator".to_string(),
                    NifValue::Ref(interpolator_id as i32),
                ),
                ("Controller".to_string(), NifValue::Ref(-1)),
            ]))]),
        );
        nif.blocks[0].set_field("Num Controlled Blocks", NifValue::UInt(1));
        let text_id = nif.add_block(
            "NiTextKeyExtraData",
            Some(IndexMap::from([(
                "Text Keys".to_string(),
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                    "Value".to_string(),
                    NifValue::String("start -name exported".to_string()),
                )]))]),
            )])),
        );

        let linearized =
            quadratic_to_linear(&mut nif, &json!({"names": ["Head"], "exact_match": true}))
                .unwrap();
        assert_eq!(linearized.len(), 1);
        assert_eq!(
            nested_u64(
                nif.blocks[data_id].get_field("Translations"),
                "Interpolation"
            ),
            Some(1)
        );
        assert_eq!(fix_exported_kf(&mut nif).len(), 2);
        assert_eq!(
            nested_value(
                value_array(nif.blocks[text_id].get_field("Text Keys")).first(),
                "Value",
            )
            .and_then(value_string),
            Some("start")
        );
        assert_eq!(optimize_animations(&mut nif).len(), 1);
        assert_eq!(
            nested_array(nif.blocks[data_id].get_field("Translations"), "Keys").len(),
            2
        );
        let removed =
            remove_controlled_blocks(&mut nif, &json!({"names": ["head"], "exact_match": true}))
                .unwrap();
        assert!(!removed.is_empty());
        assert!(value_array(nif.blocks[0].get_field("Controlled Blocks")).is_empty());
    }

    #[test]
    fn copy_controlled_blocks_copies_interpolator_data_once() {
        let mut source = animation_sequence(Vec::new());
        let data_id = source.add_block("NiTransformData", None);
        let first_interpolator = source.add_block(
            "NiTransformInterpolator",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data_id as i32),
            )])),
        );
        let second_interpolator = source.add_block(
            "NiTransformInterpolator",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data_id as i32),
            )])),
        );
        source.blocks[0].set_field(
            "Controlled Blocks",
            NifValue::Array(vec![
                controlled_entry(
                    "BoneA",
                    "NiTransformController",
                    10,
                    first_interpolator as i32,
                ),
                controlled_entry(
                    "BoneB",
                    "NiTransformController",
                    20,
                    second_interpolator as i32,
                ),
            ]),
        );
        source.blocks[0].set_field("Num Controlled Blocks", NifValue::UInt(2));
        let mut destination = animation_sequence(vec![controlled_entry(
            "Existing",
            "NiTransformController",
            1,
            -1,
        )]);
        let changes = copy_controlled_blocks_from(&mut destination, &source);
        assert_eq!(changes.len(), 2);
        let entries = value_array(destination.blocks[0].get_field("Controlled Blocks"));
        assert_eq!(entries.len(), 3);
        let copied_interpolators = &entries[1..]
            .iter()
            .map(|entry| {
                nested_value(Some(entry), "Interpolator")
                    .and_then(|value| value_ref(Some(value)))
                    .unwrap() as usize
            })
            .collect::<Vec<_>>();
        assert_ne!(copied_interpolators[0], copied_interpolators[1]);
        let copied_data = copied_interpolators
            .iter()
            .map(|id| value_ref(destination.blocks[*id].get_field("Data")).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(copied_data[0], copied_data[1]);
    }

    #[test]
    fn copy_priorities_matches_node_names_case_insensitively() {
        let source = animation_sequence(vec![controlled_entry(
            "Bip01 Head",
            "NiTransformController",
            77,
            -1,
        )]);
        let mut destination = animation_sequence(vec![controlled_entry(
            "bip01 head",
            "NiTransformController",
            1,
            -1,
        )]);

        let changes = copy_priorities_from(&mut destination, &source);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            nested_value(
                value_array(destination.blocks[0].get_field("Controlled Blocks")).first(),
                "Priority",
            )
            .and_then(|value| value_u64(Some(value))),
            Some(77)
        );
    }

    #[test]
    fn add_skeleton_blocks_copies_uncollided_bone_pose_to_death_animation() {
        let mut skeleton = NifFile::new("oblivion");
        let bone_id = skeleton.add_block("NiNode", None);
        skeleton.blocks[bone_id].set_field("Name", NifValue::String("Bip01 Arm".to_string()));
        skeleton.blocks[bone_id].set_field("Translation", NifValue::Vec3([1.0, 2.0, 3.0]));
        skeleton.blocks[bone_id].set_field(
            "Rotation",
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        skeleton.blocks[bone_id].set_field("Scale", NifValue::Float(1.25));
        skeleton.blocks[bone_id].set_field("Collision Object", NifValue::Ref(-1));
        let mut death = animation_sequence(Vec::new());

        let changes = add_skeleton_blocks_from(&mut death, &skeleton, &json!({}));
        assert_eq!(changes.len(), 1);
        let entry = &value_array(death.blocks[0].get_field("Controlled Blocks"))[0];
        assert_eq!(
            nested_value(Some(entry), "Priority").and_then(|value| value_u64(Some(value))),
            Some(99)
        );
        let interpolator = nested_value(Some(entry), "Interpolator")
            .and_then(|value| value_ref(Some(value)))
            .unwrap() as usize;
        assert_eq!(
            nested_value(
                death.blocks[interpolator].get_field("Transform"),
                "Translation"
            )
            .and_then(vec3_value),
            Some([1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn weijiesen_blow_up_rebuilds_non_accum_controller_links() {
        let mut nif = NifFile::new("fnv");
        let non_accum = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String("Bip01 NonAccum".to_string()),
            )])),
        );
        let interpolator = nif.add_block("NiTransformInterpolator", None);
        let transform_controller = nif.add_block(
            "NiTransformController",
            Some(IndexMap::from([(
                "Interpolator".to_string(),
                NifValue::Ref(interpolator as i32),
            )])),
        );
        let moving_child = nif.add_block(
            "NiNode",
            Some(IndexMap::from([
                (
                    "Name".to_string(),
                    NifValue::String("ExplosionPiece".to_string()),
                ),
                (
                    "Controller".to_string(),
                    NifValue::Ref(transform_controller as i32),
                ),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(non_accum as i32),
                NifValue::Ref(moving_child as i32),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        nif.add_block("NiMultiTargetTransformController", None);
        nif.add_block("NiDefaultAVObjectPalette", None);
        nif.add_block("NiControllerSequence", None);

        let changes = weijiesen_blow_up(&mut nif);
        assert_eq!(changes.len(), 1);
        let non_accum = nif
            .blocks
            .iter()
            .find(|block| block.get_field("Name").and_then(value_string) == Some("Bip01 NonAccum"))
            .unwrap();
        assert_eq!(value_array(non_accum.get_field("Children")).len(), 1);
        let moving_child = nif
            .blocks
            .iter()
            .find(|block| block.get_field("Name").and_then(value_string) == Some("ExplosionPiece"))
            .unwrap();
        assert_eq!(value_ref(moving_child.get_field("Controller")), Some(-1));
        let sequence = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiControllerSequence")
            .unwrap();
        let entry = value_array(sequence.get_field("Controlled Blocks"))
            .first()
            .unwrap();
        assert_eq!(
            nested_value(Some(entry), "Node Name").and_then(value_string),
            Some("ExplosionPiece")
        );
        let linked_interpolator = nested_value(Some(entry), "Interpolator")
            .and_then(|value| value_ref(Some(value)))
            .unwrap();
        assert!(linked_interpolator >= 0);
        assert_eq!(
            nif.blocks[linked_interpolator as usize].type_name,
            "NiTransformInterpolator"
        );
    }

    #[test]
    fn remove_nodes_removes_newly_unreachable_branch_only() {
        let mut nif = NifFile::new("fo4");
        let child_id = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String("RemoveMe".to_string()),
            )])),
        );
        let grandchild_id = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String("Descendant".to_string()),
            )])),
        );
        let unused_id = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String("AlreadyUnused".to_string()),
            )])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(child_id as i32)]),
        );
        nif.blocks[child_id].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(grandchild_id as i32)]),
        );
        let changes = remove_nodes(
            &mut nif,
            &json!({"names": ["removeme"], "exact_match": true}),
        )
        .unwrap();
        assert_eq!(changes.len(), 2);
        assert!(value_array(nif.blocks[0].get_field("Children")).is_empty());
        assert!(nif.blocks.iter().any(|block| {
            block
                .get_field("Name")
                .and_then(value_string)
                .is_some_and(|name| name == "AlreadyUnused")
        }));
        assert!(unused_id > grandchild_id);
    }

    #[test]
    fn remove_nodes_type_mode_matches_subtypes_and_the_root() {
        let mut nif = NifFile::new("fo4");
        assert_eq!(nif.blocks[0].type_name, "BSFadeNode");
        let changes = remove_nodes(&mut nif, &json!({"node_type": "NiNode"})).unwrap();
        assert_eq!(changes, vec!["Removed 0 BSFadeNode"]);
        assert!(nif.blocks.is_empty());
    }

    #[test]
    fn attach_parent_inserts_before_child_and_remaps_links() {
        let mut nif = NifFile::new("fo4");
        let child_id = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String("##SightingNode".to_string()),
            )])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(child_id as i32)]),
        );
        let changes = attach_parent(&mut nif, &json!({})).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            value_string(nif.blocks[1].get_field("Name").unwrap()),
            Some("##ISControl")
        );
        assert_eq!(
            value_ref(value_array(nif.blocks[0].get_field("Children")).first()),
            Some(1)
        );
        assert_eq!(
            value_ref(value_array(nif.blocks[1].get_field("Children")).first()),
            Some(2)
        );
        assert_eq!(nif.header.footer_roots, vec![0]);
    }

    #[test]
    fn morrowind_bounding_box_node_has_box_volume() {
        let mut nif = NifFile::new("morrowind");
        assert_eq!(nif.blocks[0].type_name, "NiNode");
        let changes = add_bounding_box(
            &mut nif,
            &json!({"bounding_flags": 12, "center": [1, 2, 3], "extent": [4, 5, 6]}),
        );
        assert_eq!(changes.len(), 1);
        let child_id =
            value_ref(value_array(nif.blocks[0].get_field("Children")).first()).unwrap() as usize;
        let volume = nif.blocks[child_id].get_field("Bounding Volume");
        assert_eq!(nested_u64(volume, "Collision Type"), Some(1));
        assert_eq!(
            nested_value(volume, "Box")
                .and_then(|value| nested_value(Some(value), "Extent"))
                .and_then(vec3_value),
            Some([4.0, 5.0, 6.0])
        );
    }

    #[test]
    fn morrowind_root_collision_node_bakes_shape_transform() {
        let mut nif = NifFile::new("morrowind");
        let data_id = nif.add_block(
            "NiTriShapeData",
            Some(IndexMap::from([
                ("Num Vertices".to_string(), NifValue::UInt(3)),
                ("Num Triangles".to_string(), NifValue::UInt(1)),
                (
                    "Vertices".to_string(),
                    NifValue::Array(vec![
                        NifValue::Vec3([1.0, 0.0, 0.0]),
                        NifValue::Vec3([0.0, 1.0, 0.0]),
                        NifValue::Vec3([0.0, 0.0, 1.0]),
                    ]),
                ),
            ])),
        );
        let shape_id = nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([
                ("Data".to_string(), NifValue::Ref(data_id as i32)),
                ("Translation".to_string(), NifValue::Vec3([10.0, 0.0, 0.0])),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        let changes = add_root_collision_node(&mut nif);
        assert_eq!(changes.len(), 1);
        let collision = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "RootCollisionNode")
            .unwrap();
        let collision_shape_id =
            value_ref(value_array(collision.get_field("Children")).first()).unwrap() as usize;
        let collision_data_id =
            value_ref(nif.blocks[collision_shape_id].get_field("Data")).unwrap() as usize;
        assert_eq!(
            value_array(nif.blocks[collision_data_id].get_field("Vertices"))
                .first()
                .and_then(vec3_value),
            Some([11.0, 0.0, 0.0])
        );
    }

    #[test]
    fn apply_transform_propagates_node_transform_and_bakes_geometry() {
        let mut nif = NifFile::new("oblivion");
        let node_id = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Translation".to_string(),
                NifValue::Vec3([5.0, 0.0, 0.0]),
            )])),
        );
        let data_id = nif.add_block(
            "NiTriShapeData",
            Some(IndexMap::from([
                ("Num Vertices".to_string(), NifValue::UInt(3)),
                ("Num Triangles".to_string(), NifValue::UInt(1)),
                (
                    "Vertices".to_string(),
                    NifValue::Array(vec![
                        NifValue::Vec3([1.0, 0.0, 0.0]),
                        NifValue::Vec3([0.0, 1.0, 0.0]),
                        NifValue::Vec3([0.0, 0.0, 1.0]),
                    ]),
                ),
            ])),
        );
        let shape_id = nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data_id as i32),
            )])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(node_id as i32)]),
        );
        nif.blocks[node_id].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        let changes = apply_transforms(&mut nif, &json!({}));
        assert_eq!(changes.len(), 2);
        assert_eq!(
            value_array(nif.blocks[data_id].get_field("Vertices"))
                .first()
                .and_then(vec3_value),
            Some([6.0, 0.0, 0.0])
        );
        assert_eq!(
            nif.blocks[node_id].get_field("Translation"),
            Some(&NifValue::Vec3([0.0; 3]))
        );
    }

    #[test]
    fn adjust_transform_matches_nif_add_and_name_filtering() {
        let mut nif = NifFile::new("fo4");
        let child = nif.add_block("NiNode", None);
        nif.blocks[child].set_field("Name", NifValue::String("WeaponNode".to_string()));
        nif.blocks[child].set_field("Translation", NifValue::Vec3([1.0, 2.0, 3.0]));
        nif.blocks[child].set_field("Scale", NifValue::Float(2.0));
        nif.blocks[child].set_field(
            "Rotation",
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        let changes = adjust_transform(
            &mut nif,
            &json!({
                "names": ["weapon"],
                "exact_match": false,
                "transform_mode": "add",
                "translate_x": 4.0,
                "yaw": 90.0,
                "scale": 0.5
            }),
        )
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            nif.blocks[child].get_field("Translation"),
            Some(&NifValue::Vec3([5.0, 2.0, 3.0]))
        );
        assert_eq!(
            nif.blocks[child].get_field("Scale"),
            Some(&NifValue::Float(2.5))
        );
        let NifValue::Matrix33(rotation) = nif.blocks[child].get_field("Rotation").unwrap() else {
            panic!("missing rotation")
        };
        assert!((rotation[1][2] + 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn merge_properties_relinks_identical_texture_sets() {
        let mut nif = NifFile::new("fo4");
        let first = nif.add_block("BSShaderTextureSet", None);
        let second = nif.add_block("BSShaderTextureSet", None);
        for block_id in [first, second] {
            nif.blocks[block_id].set_field(
                "Textures",
                NifValue::Array(vec![NifValue::String("textures\\same.dds".to_string())]),
            );
        }
        let first_shader = nif.add_block("BSLightingShaderProperty", None);
        let second_shader = nif.add_block("BSLightingShaderProperty", None);
        nif.blocks[first_shader].set_field("Texture Set", NifValue::Ref(first as i32));
        nif.blocks[second_shader].set_field("Texture Set", NifValue::Ref(second as i32));
        nif.blocks[0].set_field(
            "Extra Data List",
            NifValue::Array(vec![
                NifValue::Ref(first_shader as i32),
                NifValue::Ref(second_shader as i32),
            ]),
        );
        let changes =
            merge_properties(&mut nif, &json!({"property_types": ["BSShaderTextureSet"]})).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            nif.blocks
                .iter()
                .filter(|block| block.type_name == "BSShaderTextureSet")
                .count(),
            1
        );
        let refs = nif
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSLightingShaderProperty")
            .map(|block| value_ref(block.get_field("Texture Set")).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(refs[0], refs[1]);
    }

    #[test]
    fn fo3_merge_properties_ignores_names_and_material_specular_by_default() {
        let mut nif = NifFile::new("fnv");
        let first = nif.add_block(
            "NiMaterialProperty",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("First".to_string())),
                (
                    "Specular Color".to_string(),
                    NifValue::Color3([1.0, 0.0, 0.0]),
                ),
                ("Glossiness".to_string(), NifValue::Float(10.0)),
            ])),
        );
        let second = nif.add_block(
            "NiMaterialProperty",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Second".to_string())),
                (
                    "Specular Color".to_string(),
                    NifValue::Color3([0.0, 1.0, 0.0]),
                ),
                ("Glossiness".to_string(), NifValue::Float(10.0)),
            ])),
        );
        nif.blocks[0].set_field(
            "Extra Data List",
            NifValue::Array(vec![
                NifValue::Ref(first as i32),
                NifValue::Ref(second as i32),
            ]),
        );
        let changes = merge_properties(&mut nif, &json!({})).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            nif.blocks
                .iter()
                .filter(|block| block.type_name == "NiMaterialProperty")
                .count(),
            1
        );
    }

    #[test]
    fn group_shapes_uses_diffuse_texture_basename() {
        let mut nif = NifFile::new("fo4");
        let first = nif.add_block("BSTriShape", None);
        let second = nif.add_block("BSTriShape", None);
        let texture_set = nif.add_block("BSShaderTextureSet", None);
        nif.blocks[texture_set].set_field(
            "Textures",
            NifValue::Array(vec![NifValue::String(
                "textures\\architecture\\wall_d.dds".to_string(),
            )]),
        );
        let shader = nif.add_block("BSLightingShaderProperty", None);
        nif.blocks[shader].set_field("Texture Set", NifValue::Ref(texture_set as i32));
        for shape_id in [first, second] {
            nif.blocks[shape_id].set_field("Shader Property", NifValue::Ref(shader as i32));
        }
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(first as i32),
                NifValue::Ref(second as i32),
            ]),
        );
        let changes = group_shapes(&mut nif, &json!({}));
        assert_eq!(changes.len(), 1);
        let group_id =
            value_ref(value_array(nif.blocks[0].get_field("Children")).first()).unwrap() as usize;
        assert_eq!(nif.blocks[group_id].type_name, "NiNode");
        assert_eq!(
            nif.blocks[group_id]
                .get_field("Name")
                .and_then(value_string),
            Some("wall_d.dds")
        );
        assert_eq!(
            value_array(nif.blocks[group_id].get_field("Children")).len(),
            2
        );
    }

    #[test]
    fn vertex_paint_adds_legacy_colors_and_replaces_modern_byte_colors() {
        let mut legacy = NifFile::new("oblivion");
        let data = legacy.add_block(
            "NiTriShapeData",
            Some(IndexMap::from([
                (
                    "Vertices".to_string(),
                    NifValue::Array(vec![
                        NifValue::Vec3([0.0, 0.0, 0.0]),
                        NifValue::Vec3([1.0, 0.0, 0.0]),
                    ]),
                ),
                ("Has Vertex Colors".to_string(), NifValue::Bool(false)),
                ("Vertex Colors".to_string(), NifValue::Array(Vec::new())),
            ])),
        );
        legacy.add_block(
            "NiTriShape",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data as i32),
            )])),
        );
        let changes = vertex_paint(
            &mut legacy,
            &json!({"paint_mode": "set", "color": "000000FF", "add_if_missing": true}),
        )
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            value_array(legacy.blocks[data].get_field("Vertex Colors")).len(),
            2
        );

        let mut modern = NifFile::new("fo4");
        let shape = modern.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                ("Vertex Desc".to_string(), NifValue::UInt(0x20 << 44)),
                (
                    "Vertex Data".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                        "Vertex Colors".to_string(),
                        NifValue::Struct(IndexMap::from([
                            ("r".to_string(), NifValue::UInt(255)),
                            ("g".to_string(), NifValue::UInt(0)),
                            ("b".to_string(), NifValue::UInt(0)),
                            ("a".to_string(), NifValue::UInt(255)),
                        ])),
                    )]))]),
                ),
            ])),
        );
        vertex_paint(
            &mut modern,
            &json!({
                "paint_mode": "replace",
                "color": "FF0000FF",
                "replacement_color": "00FF00FF"
            }),
        )
        .unwrap();
        let painted = nested_value(
            value_array(modern.blocks[shape].get_field("Vertex Data")).first(),
            "Vertex Colors",
        )
        .and_then(|value| read_vertex_color(value, true))
        .unwrap();
        assert!(same_color(painted, [0.0, 1.0, 0.0, 1.0]));
    }

    #[test]
    fn legacy_vertex_paint_uses_existing_arrays_independently_of_the_color_flag() {
        let mut nif = NifFile::new("oblivion");
        let data = nif.add_block(
            "NiTriShapeData",
            Some(IndexMap::from([
                (
                    "Vertices".to_string(),
                    NifValue::Array(vec![NifValue::Vec3([0.0, 0.0, 0.0])]),
                ),
                ("Has Vertex Colors".to_string(), NifValue::Bool(false)),
                (
                    "Vertex Colors".to_string(),
                    NifValue::Array(vec![NifValue::Color4([1.0; 4])]),
                ),
            ])),
        );
        nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data as i32),
            )])),
        );
        vertex_paint(&mut nif, &json!({"paint_mode": "set", "color": "000000FF"})).unwrap();
        assert_eq!(
            nif.blocks[data].get_field("Has Vertex Colors"),
            Some(&NifValue::Bool(false))
        );
        assert!(same_color(
            read_vertex_color(
                value_array(nif.blocks[data].get_field("Vertex Colors"))
                    .first()
                    .unwrap(),
                false
            )
            .unwrap(),
            [0.0, 0.0, 0.0, 1.0]
        ));

        nif.blocks[data].set_field("Has Vertex Colors", NifValue::Bool(true));
        nif.blocks[data].set_field("Vertex Colors", NifValue::Array(Vec::new()));
        vertex_paint(&mut nif, &json!({"paint_mode": "set", "color": "FFFFFFFF"})).unwrap();
        assert_eq!(
            value_array(nif.blocks[data].get_field("Vertex Colors")).len(),
            1
        );
    }

    #[test]
    fn vertex_paint_only_handles_skin_partitions_in_skyrim_se() {
        let mut nif = NifFile::new("fnv");
        let partition = nif.add_block(
            "NiSkinPartition",
            Some(IndexMap::from([
                ("Vertex Desc".to_string(), NifValue::UInt(0x20 << 44)),
                ("Vertex Data".to_string(), NifValue::Array(Vec::new())),
            ])),
        );
        let changes =
            vertex_paint(&mut nif, &json!({"paint_mode": "set", "color": "000000FF"})).unwrap();
        assert!(changes.is_empty());
        assert!(value_array(nif.blocks[partition].get_field("Vertex Data")).is_empty());
    }

    #[test]
    fn unskin_mesh_removes_bones_and_legacy_skinning_flags() {
        let mut nif = NifFile::new("fnv");
        let bone = nif.add_block(
            "NiNode",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String("Bip01 Spine".to_string()),
            )])),
        );
        let skin = nif.add_block(
            "NiSkinInstance",
            Some(IndexMap::from([(
                "Bones".to_string(),
                NifValue::Array(vec![NifValue::Ref(bone as i32)]),
            )])),
        );
        let shader = nif.add_block(
            "BSShaderPPLightingProperty",
            Some(IndexMap::from([(
                "Shader Flags".to_string(),
                NifValue::UInt(1 << 1),
            )])),
        );
        let shape = nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Body".to_string())),
                ("Skin Instance".to_string(), NifValue::Ref(skin as i32)),
                (
                    "Properties".to_string(),
                    NifValue::Array(vec![NifValue::Ref(shader as i32)]),
                ),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(bone as i32),
                NifValue::Ref(shape as i32),
            ]),
        );
        let changes = unskin_mesh(&mut nif);
        assert!(
            changes
                .iter()
                .any(|change| change == "Removed 1 bone node(s)")
        );
        assert!(!nif.blocks.iter().any(|block| {
            block.get_field("Name").and_then(value_string) == Some("Bip01 Spine")
        }));
        let shape = nif
            .blocks
            .iter()
            .find(|block| block.get_field("Name").and_then(value_string) == Some("Body"))
            .unwrap();
        assert_eq!(value_ref(shape.get_field("Skin Instance")), Some(-1));
        let shader = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSShaderPPLightingProperty")
            .unwrap();
        assert_eq!(value_u64(shader.get_field("Shader Flags")), Some(0));
    }

    #[test]
    fn merge_shapes_combines_modern_vertices_and_offsets_triangles() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::String("MergeNode".to_string()));
        let make_vertices = |offset: f32| {
            NifValue::Array(
                [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                    .into_iter()
                    .map(|mut vertex| {
                        vertex[0] += offset;
                        NifValue::Struct(IndexMap::from([(
                            "Vertex".to_string(),
                            NifValue::Vec3(vertex),
                        )]))
                    })
                    .collect(),
            )
        };
        let triangle = || {
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                ("v1".to_string(), NifValue::UInt(0)),
                ("v2".to_string(), NifValue::UInt(1)),
                ("v3".to_string(), NifValue::UInt(2)),
            ]))])
        };
        let first = nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("First".to_string())),
                ("Vertex Data".to_string(), make_vertices(0.0)),
                ("Triangles".to_string(), triangle()),
                ("Num Vertices".to_string(), NifValue::UInt(3)),
                ("Num Triangles".to_string(), NifValue::UInt(1)),
            ])),
        );
        let second = nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Second".to_string())),
                ("Vertex Data".to_string(), make_vertices(2.0)),
                ("Triangles".to_string(), triangle()),
                ("Num Vertices".to_string(), NifValue::UInt(3)),
                ("Num Triangles".to_string(), NifValue::UInt(1)),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(first as i32),
                NifValue::Ref(second as i32),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));

        let changes = merge_shapes(
            &mut nif,
            &json!({"names": ["MergeNode"], "exact_match": true}),
        )
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(value_array(nif.blocks[0].get_field("Children")).len(), 1);
        let merged = nif
            .blocks
            .iter()
            .find(|block| block.get_field("Name").and_then(value_string) == Some("First"))
            .unwrap();
        assert_eq!(value_array(merged.get_field("Vertex Data")).len(), 6);
        assert_eq!(triangles(merged), vec![[0, 1, 2], [3, 4, 5]]);
    }

    #[test]
    fn copy_geometry_blocks_preserves_destination_modern_links() {
        let mut source = NifFile::new("fo4");
        source.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Shape".to_string())),
                (
                    "Vertex Data".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                        "Vertex".to_string(),
                        NifValue::Vec3([9.0, 8.0, 7.0]),
                    )]))]),
                ),
                ("Shader Property".to_string(), NifValue::Ref(-1)),
            ])),
        );
        let mut destination = NifFile::new("fo4");
        let shader = destination.add_block("BSLightingShaderProperty", None);
        let shape = destination.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Shape".to_string())),
                (
                    "Vertex Data".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                        "Vertex".to_string(),
                        NifValue::Vec3([0.0, 0.0, 0.0]),
                    )]))]),
                ),
                ("Shader Property".to_string(), NifValue::Ref(shader as i32)),
            ])),
        );

        let changes =
            copy_geometry_blocks_from(&mut destination, &source, true, false, false, false);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            value_ref(destination.blocks[shape].get_field("Shader Property")),
            Some(shader as i32)
        );
        assert_eq!(
            nested_value(
                value_array(destination.blocks[shape].get_field("Vertex Data")).first(),
                "Vertex",
            )
            .and_then(vec3_value),
            Some([9.0, 8.0, 7.0])
        );
    }

    #[test]
    fn copy_geometry_blocks_removes_obsolete_oblivion_tangent_data() {
        let mut source = NifFile::new("oblivion");
        let source_data = source.add_block(
            "NiTriShapeData",
            Some(IndexMap::from([(
                "Vertices".to_string(),
                NifValue::Array(vec![NifValue::Vec3([9.0, 8.0, 7.0])]),
            )])),
        );
        let source_shape = source.add_block(
            "NiTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Shape".to_string())),
                ("Data".to_string(), NifValue::Ref(source_data as i32)),
                ("Num Extra Data List".to_string(), NifValue::UInt(0)),
                ("Extra Data List".to_string(), NifValue::Array(Vec::new())),
            ])),
        );
        source.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(source_shape as i32)]),
        );

        let mut destination = NifFile::new("oblivion");
        let destination_data = destination.add_block(
            "NiTriShapeData",
            Some(IndexMap::from([(
                "Vertices".to_string(),
                NifValue::Array(vec![NifValue::Vec3([0.0, 0.0, 0.0])]),
            )])),
        );
        let destination_shape = destination.add_block(
            "NiTriShape",
            Some(IndexMap::from([
                ("Name".to_string(), NifValue::String("Shape".to_string())),
                ("Data".to_string(), NifValue::Ref(destination_data as i32)),
                ("Num Extra Data List".to_string(), NifValue::UInt(0)),
                ("Extra Data List".to_string(), NifValue::Array(Vec::new())),
            ])),
        );
        write_oblivion_tangents(
            &mut destination,
            destination_shape,
            &[[1.0, 0.0, 0.0]],
            &[[0.0, 1.0, 0.0]],
        );
        destination.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(destination_shape as i32)]),
        );

        let changes =
            copy_geometry_blocks_from(&mut destination, &source, true, false, false, false);
        assert_eq!(changes.len(), 1);
        assert!(
            destination
                .blocks
                .iter()
                .all(|block| block.type_name != "NiBinaryExtraData")
        );
        assert_eq!(
            value_array(destination.blocks[destination_shape].get_field("Extra Data List")).len(),
            0
        );
        assert_eq!(
            value_array(destination.blocks[destination_data].get_field("Vertices"))
                .first()
                .and_then(vec3_value),
            Some([9.0, 8.0, 7.0])
        );
    }

    #[test]
    fn optimize_mesh_reorders_modern_vertices_for_first_use() {
        let mut nif = NifFile::new("fo4");
        let shape = nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                (
                    "Vertex Data".to_string(),
                    NifValue::Array(
                        [[10.0, 0.0, 0.0], [20.0, 0.0, 0.0], [30.0, 0.0, 0.0]]
                            .into_iter()
                            .map(|position| {
                                NifValue::Struct(IndexMap::from([(
                                    "Vertex".to_string(),
                                    NifValue::Vec3(position),
                                )]))
                            })
                            .collect(),
                    ),
                ),
                (
                    "Triangles".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                        ("v1".to_string(), NifValue::UInt(2)),
                        ("v2".to_string(), NifValue::UInt(0)),
                        ("v3".to_string(), NifValue::UInt(1)),
                    ]))]),
                ),
            ])),
        );
        let changes = optimize_mesh(&mut nif, &json!({"vertex_fetch": true})).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(triangles(&nif.blocks[shape]), vec![[0, 1, 2]]);
        assert_eq!(positions(&nif.blocks[shape])[0], [30.0, 0.0, 0.0]);
    }

    #[test]
    fn update_mopp_code_writes_compiled_data_origin_and_scale() {
        let mut nif = NifFile::new("fnv");
        nif.blocks[0].set_field("Children", NifValue::Array(vec![NifValue::Ref(1)]));

        let mut mopp = NifBlock::new(1, "bhkMoppBvTreeShape");
        mopp.set_field("Shape", NifValue::Ref(2));
        mopp.set_field("Scale", NifValue::Float(1.0));
        mopp.set_field(
            "MOPP Code",
            NifValue::Struct(IndexMap::from([
                ("Data Size".to_string(), NifValue::UInt(0)),
                ("Offset".to_string(), NifValue::Vec4([0.0; 4])),
                ("Build Type".to_string(), NifValue::UInt(0)),
                ("Data".to_string(), NifValue::Bytes(Vec::new())),
            ])),
        );

        let mut packed = NifBlock::new(2, "bhkPackedNiTriStripsShape");
        packed.set_field("Radius", NifValue::Float(0.1));
        packed.set_field("Data", NifValue::Ref(3));
        packed.set_field(
            "Sub Shapes",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                "Num Vertices".to_string(),
                NifValue::UInt(4),
            )]))]),
        );

        let triangle = |v1, v2, v3| {
            NifValue::Struct(IndexMap::from([(
                "Triangle".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("v1".to_string(), NifValue::UInt(v1)),
                    ("v2".to_string(), NifValue::UInt(v2)),
                    ("v3".to_string(), NifValue::UInt(v3)),
                ])),
            )]))
        };
        let mut data = NifBlock::new(3, "hkPackedNiTriStripsData");
        data.set_field(
            "Vertices",
            NifValue::Array(vec![
                NifValue::Vec3([0.0, 0.0, 0.0]),
                NifValue::Vec3([1.0, 0.0, 0.0]),
                NifValue::Vec3([0.0, 1.0, 0.0]),
                NifValue::Vec3([1.0, 1.0, 0.0]),
            ]),
        );
        data.set_field(
            "Triangles",
            NifValue::Array(vec![triangle(0, 1, 2), triangle(2, 1, 3)]),
        );
        nif.blocks.extend([mopp, packed, data]);

        let changes = update_mopp_code(&mut nif).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(value_f64(nif.blocks[1].get_field("Scale")).unwrap() > 1.0);
        let code = nif.blocks[1].get_field("MOPP Code");
        let bytes = nested_value(code, "Data").unwrap();
        let NifValue::Bytes(bytes) = bytes else {
            panic!("expected MOPP bytecode");
        };
        assert!(!bytes.is_empty());
        assert_eq!(nested_u64(code, "Data Size"), Some(bytes.len() as u64));
        assert_eq!(
            nested_value(code, "Offset").and_then(vec4_xyz),
            Some([-0.1, -0.1, -0.1])
        );
        assert!(changes[0].ends_with("across 1 subshapes"));
    }

    #[test]
    fn update_mopp_code_accepts_subshapes_on_packed_data() {
        let mut nif = NifFile::new("oblivion");
        let mut mopp = NifBlock::new(1, "bhkMoppBvTreeShape");
        mopp.set_field("Shape", NifValue::Ref(2));
        mopp.set_field(
            "MOPP Code",
            NifValue::Struct(IndexMap::from([
                ("Data Size".to_string(), NifValue::UInt(0)),
                ("Offset".to_string(), NifValue::Vec4([0.0; 4])),
                ("Data".to_string(), NifValue::Bytes(Vec::new())),
            ])),
        );
        let mut packed = NifBlock::new(2, "bhkPackedNiTriStripsShape");
        packed.set_field("Data", NifValue::Ref(3));
        let mut data = NifBlock::new(3, "hkPackedNiTriStripsData");
        data.set_field(
            "Sub Shapes",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                "Num Vertices".to_string(),
                NifValue::UInt(3),
            )]))]),
        );
        data.set_field(
            "Vertices",
            NifValue::Array(vec![
                NifValue::Vec3([0.0, 0.0, 0.0]),
                NifValue::Vec3([1.0, 0.0, 0.0]),
                NifValue::Vec3([0.0, 1.0, 0.0]),
            ]),
        );
        data.set_field(
            "Triangles",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                "Triangle".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("v1".to_string(), NifValue::UInt(0)),
                    ("v2".to_string(), NifValue::UInt(1)),
                    ("v3".to_string(), NifValue::UInt(2)),
                ])),
            )]))]),
        );
        nif.blocks.extend([mopp, packed, data]);

        let changes = update_mopp_code(&mut nif).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].ends_with("across 1 subshapes"));
    }

    #[test]
    fn optimize_mesh_triangulates_legacy_points() {
        let mut nif = NifFile::new("oblivion");
        let data = nif.add_block(
            "NiTriStripsData",
            Some(IndexMap::from([
                (
                    "Vertices".to_string(),
                    NifValue::Array(vec![
                        NifValue::Vec3([0.0, 0.0, 0.0]),
                        NifValue::Vec3([1.0, 0.0, 0.0]),
                        NifValue::Vec3([0.0, 1.0, 0.0]),
                        NifValue::Vec3([1.0, 1.0, 0.0]),
                    ]),
                ),
                (
                    "Points".to_string(),
                    NifValue::Array(vec![NifValue::Array(vec![
                        NifValue::UInt(0),
                        NifValue::UInt(1),
                        NifValue::UInt(2),
                        NifValue::UInt(3),
                    ])]),
                ),
            ])),
        );
        let shape = nif.add_block(
            "NiTriStrips",
            Some(IndexMap::from([(
                "Data".to_string(),
                NifValue::Ref(data as i32),
            )])),
        );
        optimize_mesh(&mut nif, &json!({"triangulate": true})).unwrap();
        assert_eq!(nif.blocks[shape].type_name, "NiTriShape");
        assert_eq!(nif.blocks[data].type_name, "NiTriShapeData");
        assert_eq!(triangles(&nif.blocks[data]), vec![[0, 1, 2], [2, 1, 3]]);
    }

    #[test]
    fn add_transform_data_copies_interpolator_pose_to_end_keys() {
        let mut nif = NifFile::new("fnv");
        nif.blocks[0].set_field("Stop Time", NifValue::Float(2.0));
        let interpolator = nif.add_block("NiTransformInterpolator", None);
        nif.blocks[interpolator].set_field("Data", NifValue::Ref(-1));
        nif.blocks[interpolator].set_field(
            "Transform",
            NifValue::Struct(IndexMap::from([
                ("Translation".to_string(), NifValue::Vec3([1.0, 2.0, 3.0])),
                (
                    "Rotation".to_string(),
                    NifValue::Quaternion([0.0, 0.0, 0.0, 1.0]),
                ),
                ("Scale".to_string(), NifValue::Float(1.0)),
            ])),
        );
        let changes = add_transform_data(&mut nif, &json!({})).unwrap();
        assert_eq!(changes.len(), 1);
        let data_id = value_ref(nif.blocks[interpolator].get_field("Data")).unwrap() as usize;
        assert_eq!(
            value_array(nif.blocks[data_id].get_field("Quaternion Keys")).len(),
            2
        );
        let translations = nif.blocks[data_id].get_field("Translations");
        assert_eq!(nested_u64(translations, "Num Keys"), Some(2));
        let end_time = nested_array(translations, "Keys")
            .get(1)
            .and_then(|key| nested_value(Some(key), "Time"))
            .and_then(|value| value_f64(Some(value)));
        assert_eq!(end_time, Some(2.0));
    }

    #[test]
    fn add_headtracking_anim_uses_existing_head_priority() {
        let mut nif = NifFile::new("fnv");
        nif.blocks[0].type_name = "NiControllerSequence".to_string();
        nif.blocks[0].set_field("Stop Time", NifValue::Float(3.0));
        nif.blocks[0].set_field(
            "Controlled Blocks",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                (
                    "Node Name".to_string(),
                    NifValue::String("Bip01 Head".to_string()),
                ),
                ("Priority".to_string(), NifValue::UInt(77)),
                (
                    "Controller Type".to_string(),
                    NifValue::String("NiTransformController".to_string()),
                ),
            ]))]),
        );
        let changes = add_headtracking_anim(&mut nif, &json!({}));
        assert_eq!(changes.len(), 1);
        let entries = value_array(nif.blocks[0].get_field("Controlled Blocks"));
        assert_eq!(entries.len(), 2);
        assert_eq!(nested_u64(entries.get(1), "Priority"), Some(77));
        assert_eq!(
            nested_value(entries.get(1), "Controller ID").and_then(value_string),
            Some("HeadTrack")
        );
        let interpolator_id = nested_value(entries.get(1), "Interpolator")
            .and_then(|value| value_ref(Some(value)))
            .unwrap() as usize;
        let data_id = value_ref(nif.blocks[interpolator_id].get_field("Data")).unwrap() as usize;
        assert_eq!(
            nested_array(nif.blocks[data_id].get_field("Data"), "Keys").len(),
            4
        );
    }

    #[test]
    fn add_facial_anim_builds_head_and_modifier_blocks() {
        let mut nif = NifFile::new("fnv");
        nif.blocks[0].type_name = "NiControllerSequence".to_string();
        nif.blocks[0].set_field("Controlled Blocks", NifValue::Array(Vec::new()));
        let changes = add_facial_anim(
            &mut nif,
            &json!({"facial_mods": ["99 Aah 0.466667 0 1.499999 1"]}),
        )
        .unwrap();
        assert_eq!(changes.len(), 2);
        let entries = value_array(nif.blocks[0].get_field("Controlled Blocks"));
        assert_eq!(entries.len(), 2);
        assert_eq!(
            nested_value(entries.first(), "Node Name").and_then(value_string),
            Some("HeadAnims")
        );
        assert_eq!(
            nested_value(entries.get(1), "Interpolator ID").and_then(value_string),
            Some("Aah")
        );
        let interpolator_id = nested_value(entries.get(1), "Interpolator")
            .and_then(|value| value_ref(Some(value)))
            .unwrap() as usize;
        let data_id = value_ref(nif.blocks[interpolator_id].get_field("Data")).unwrap() as usize;
        assert_eq!(
            nested_array(nif.blocks[data_id].get_field("Data"), "Keys").len(),
            2
        );
    }

    #[test]
    fn add_lod_node_moves_root_shapes_and_writes_ranges() {
        let mut nif = NifFile::new("fnv");
        let first = nif.add_block("NiTriShape", None);
        let second = nif.add_block("NiTriShape", None);
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(first as i32),
                NifValue::Ref(second as i32),
            ]),
        );
        let changes = add_lod_node(
            &mut nif,
            &json!({"lod_data": "range", "extents": [100.0, 200.0]}),
        )
        .unwrap();
        assert_eq!(changes.len(), 5);
        assert!(
            changes
                .iter()
                .any(|change| change.ends_with(": Updated LOD levels"))
        );
        assert_eq!(nif.blocks[1].type_name, "NiLODNode");
        assert_eq!(
            value_array(nif.blocks[0].get_field("Children")),
            &[NifValue::Ref(1)]
        );
        assert_eq!(
            value_array(nif.blocks[1].get_field("Children")),
            &[NifValue::Ref(2), NifValue::Ref(3)]
        );
        let data_id = value_ref(nif.blocks[1].get_field("LOD Level Data")).unwrap() as usize;
        let levels = value_array(nif.blocks[data_id].get_field("LOD Levels"));
        assert_eq!(levels.len(), 2);
        assert_eq!(
            nested_value(levels.first(), "Far Extent").and_then(|value| value_f64(Some(value))),
            Some(100.0)
        );

        let changes = add_lod_node(
            &mut nif,
            &json!({"lod_data": "range", "extents": [300.0, 400.0]}),
        )
        .unwrap();
        assert_eq!(changes, vec![format!("{data_id}: Updated LOD levels")]);
        assert_eq!(
            nested_value(
                value_array(nif.blocks[data_id].get_field("LOD Levels")).first(),
                "Far Extent"
            )
            .and_then(|value| value_f64(Some(value))),
            Some(300.0)
        );
    }
}
