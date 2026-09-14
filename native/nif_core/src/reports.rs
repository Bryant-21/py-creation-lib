use std::collections::HashSet;
use std::path::Path;

use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::model::{NifBlock, NifFile, NifValue};
use crate::schema::SCHEMA;

pub fn report_nif_file(path: &Path, processor: &str, options_json: &str) -> Result<Value, String> {
    let nif = NifFile::load(path).map_err(|error| error.to_string())?;
    let options = if options_json.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(options_json).map_err(|error| error.to_string())?
    };
    let data = match processor {
        "analyze-mesh" => analyze_mesh(&nif, &options)?,
        "transform-information" => transform_information(&nif, &options),
        "havok-information" => havok_information(&nif, &options),
        "find-unwelded-vertices" => find_unwelded_vertices(&nif, &options)?,
        "find-excessive-draw-calls" => find_excessive_draw_calls(&nif, &options),
        "find-uvs" => find_uvs(&nif, &options)?,
        _ => return Err(format!("Unsupported NIF report processor: {processor}")),
    };
    Ok(json!({
        "processor": processor,
        "path": path,
        "game": crate::validation::nif_game_label(&nif),
        "data": data,
    }))
}

fn analyze_mesh(nif: &NifFile, options: &Value) -> Result<Value, String> {
    let cache_size = option_u64(options, "cache_size").unwrap_or(16);
    if !(1..=128).contains(&cache_size) {
        return Err("cache_size must be between 1 and 128".to_string());
    }
    let per_shape = option_bool(options, "per_shape").unwrap_or(false);
    let threshold = option_bool(options, "threshold").unwrap_or(true);
    let acmr_threshold = option_f64(options, "acmr").unwrap_or(1.5);
    let atvr_threshold = option_f64(options, "atvr").unwrap_or(1.5);
    let vertex_threshold = option_u64(options, "vertices").unwrap_or(0) as usize;
    let geometries = rendered_geometry(nif);
    let mut entries = Vec::new();
    let mut totals = (0usize, 0usize, 0.0f64, 0.0f64, 0.0f64);

    for geometry in geometries {
        if geometry.indices.is_empty() || geometry.vertex_count == 0 {
            continue;
        }
        let cache = meshopt::analyze_vertex_cache(
            &geometry.indices,
            geometry.vertex_count,
            cache_size as u32,
            0,
            0,
        );
        let fetch = meshopt::analyze_vertex_fetch(&geometry.indices, geometry.vertex_count, 12);
        let triangles = geometry.indices.len() / 3;
        let acmr = round_tenth(cache.acmr as f64);
        let atvr = round_tenth(cache.atvr as f64);
        let overfetch = round_tenth(fetch.overfetch as f64);
        let matched = !threshold
            || acmr > acmr_threshold
            || atvr > atvr_threshold
            || geometry.vertex_count > vertex_threshold;
        if per_shape && matched {
            entries.push(json!({
                "block_id": geometry.block_id,
                "name": geometry.name,
                "vertices": geometry.vertex_count,
                "triangles": triangles,
                "acmr": acmr,
                "atvr": atvr,
                "overfetch": overfetch,
            }));
        }
        totals.0 += geometry.vertex_count;
        totals.1 += triangles;
        totals.2 += triangles as f64 * cache.acmr as f64;
        totals.3 += triangles as f64 * cache.atvr as f64;
        totals.4 += triangles as f64 * fetch.overfetch as f64;
    }

    let denominator = totals.1.max(1) as f64;
    let acmr = round_tenth(totals.2 / denominator);
    let atvr = round_tenth(totals.3 / denominator);
    let overfetch = round_tenth(totals.4 / denominator);
    let matched =
        !threshold || acmr > acmr_threshold || atvr > atvr_threshold || totals.0 > vertex_threshold;
    Ok(json!({
        "cache_size": cache_size,
        "entries": entries,
        "summary": {
            "vertices": totals.0,
            "triangles": totals.1,
            "acmr": acmr,
            "atvr": atvr,
            "overfetch": overfetch,
            "matched": matched,
        }
    }))
}

fn transform_information(nif: &NifFile, options: &Value) -> Value {
    let translation = option_bool(options, "translation").unwrap_or(true);
    let rotation = option_bool(options, "rotation").unwrap_or(true);
    let scale = option_bool(options, "scale").unwrap_or(true);
    let skip_empty = option_bool(options, "skip_empty").unwrap_or(true);
    let mut entries = Vec::new();

    for block in &nif.blocks {
        let av_object = SCHEMA.is_subtype_of(&block.type_name, "NiAVObject");
        if !av_object && !SCHEMA.is_subtype_of(&block.type_name, "bhkRigidBodyT") {
            continue;
        }
        if skip_empty && av_object && is_empty_node(nif, block) {
            continue;
        }
        let transform = block
            .get_field("Transform")
            .and_then(as_struct)
            .unwrap_or(&block.fields);
        let translation_value = named_value(transform, "Translation");
        let rotation_value = named_value(transform, "Rotation");
        let scale_value = named_value(transform, "Scale");
        let has_translation = translation_value.is_some_and(|value| !value_is_zero_vector(value));
        let has_rotation = rotation_value.is_some_and(|value| !value_is_identity_matrix(value));
        let has_scale = scale_value
            .and_then(|value| value_f64(Some(value)))
            .is_some_and(|value| (value * 1000.0).round() / 1000.0 != 1.0);
        if !(translation && has_translation || rotation && has_rotation || scale && has_scale) {
            continue;
        }
        entries.push(json!({
            "block_id": block.block_id,
            "block_type": block.type_name,
            "name": block_name(block),
            "translation": (translation && has_translation).then(|| translation_value.map(nif_value_json)).flatten(),
            "rotation": (rotation && has_rotation).then(|| rotation_value.map(nif_value_json)).flatten(),
            "scale": (scale && has_scale).then(|| scale_value.and_then(|value| value_f64(Some(value)))).flatten(),
        }));
    }
    json!({"entries": entries})
}

fn havok_information(nif: &NifFile, options: &Value) -> Value {
    let fields = options
        .get("fields")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let per_object = option_bool(options, "per_object").unwrap_or(true);
    let mut entries = Vec::new();
    let mut statics = 0usize;
    let mut dynamics = 0usize;
    let mut total_mass = 0.0f64;

    for collision in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "bhkCollisionObject"))
    {
        let Some(body) = referenced_block(nif, collision.get_field("Body")) else {
            continue;
        };
        let target = referenced_block(nif, collision.get_field("Target"));
        let shape = referenced_block(nif, rigid_body_value(body, "Shape"));
        let shape = shape
            .filter(|shape| shape.type_name == "bhkTransformShape")
            .and_then(|shape| referenced_block(nif, shape.get_field("Shape")))
            .or(shape);
        let mass = value_f64(rigid_body_value(body, "Mass")).unwrap_or(0.0);
        let dynamic = rigid_body_is_dynamic(nif, body);
        if dynamic {
            dynamics += 1;
        } else {
            statics += 1;
        }
        total_mass += mass;
        if per_object {
            let selected = fields
                .iter()
                .filter_map(|field| {
                    rigid_body_value(body, field)
                        .map(|value| (field.to_string(), nif_value_json(value)))
                })
                .collect::<serde_json::Map<_, _>>();
            entries.push(json!({
                "collision_block_id": collision.block_id,
                "body_block_id": body.block_id,
                "target": target.map(block_name).unwrap_or_else(|| "<No target>".to_string()),
                "mass": mass,
                "layer": rigid_body_nested_u64(body, "Havok Filter", "Layer"),
                "shape_type": shape.map(|shape| shape.type_name.as_str()).unwrap_or("<No shape>"),
                "dynamic": dynamic,
                "fields": selected,
            }));
        }
    }
    json!({
        "entries": entries,
        "summary": {"static": statics, "dynamic": dynamics, "total_mass": total_mass}
    })
}

fn find_unwelded_vertices(nif: &NifFile, options: &Value) -> Result<Value, String> {
    let distance = option_f64(options, "distance").unwrap_or(0.1);
    if !distance.is_finite() || distance <= 0.0 {
        return Err("distance must be a positive finite number".to_string());
    }
    let skip_same = option_bool(options, "skip_same").unwrap_or(false);
    let report_vertices = option_bool(options, "report_vertices").unwrap_or(false);
    let distance_squared = distance * distance;
    let mut entries = Vec::new();

    for (block, vertices) in vertex_blocks(nif) {
        let mut pairs = Vec::new();
        let mut count = 0usize;
        for left in 0..vertices.len().saturating_sub(1) {
            for right in left + 1..vertices.len() {
                let same = vertices[left] == vertices[right];
                if skip_same && same {
                    continue;
                }
                let delta = [
                    vertices[right][0] - vertices[left][0],
                    vertices[right][1] - vertices[left][1],
                    vertices[right][2] - vertices[left][2],
                ];
                let squared = delta.iter().map(|value| value * value).sum::<f64>();
                if same || squared <= distance_squared {
                    count += 1;
                    if report_vertices {
                        pairs.push(json!({
                            "left": left,
                            "left_position": vertices[left],
                            "right": right,
                            "right_position": vertices[right],
                        }));
                    }
                }
            }
        }
        if count > 0 {
            entries.push(json!({
                "block_id": block.block_id,
                "block_type": block.type_name,
                "name": block_name(block),
                "vertices": vertices.len(),
                "unwelded_pairs": count,
                "percent": (count as f64 / vertices.len().max(1) as f64 * 100.0).round() as u64,
                "pairs": pairs,
            }));
        }
    }
    Ok(json!({"distance": distance, "entries": entries}))
}

fn find_excessive_draw_calls(nif: &NifFile, options: &Value) -> Value {
    let threshold = option_u64(options, "threshold").unwrap_or(10) as usize;
    let mut markers = HashSet::new();
    for node in nif.blocks.iter().filter(|block| {
        SCHEMA.is_subtype_of(&block.type_name, "NiNode")
            && block_name(block)
                .to_ascii_lowercase()
                .starts_with("editormarker")
    }) {
        collect_descendants(nif, node.block_id, &mut markers);
    }
    let mut entries = Vec::new();
    let mut total = 0usize;
    for shape in &nif.blocks {
        if markers.contains(&shape.block_id)
            || !matches!(shape.type_name.as_str(), "NiTriShape" | "NiTriStrips")
        {
            continue;
        }
        let shapes = if shape.type_name == "NiTriStrips" {
            referenced_block(nif, shape.get_field("Data"))
                .and_then(|data| value_u64(data.get_field("Num Strips")))
                .unwrap_or(1) as usize
        } else {
            1
        };
        let flags = geometry_shader(nif, shape)
            .and_then(|shader| value_u64(shader.get_field("Shader Flags")))
            .unwrap_or(0);
        let mut passes = 1usize;
        if flags & ((1 << 7) | (1 << 17) | (1 << 21)) != 0 {
            passes += 1;
        }
        if flags & (1 << 10) != 0 {
            passes += 1;
        }
        let calls = shapes * passes;
        total += calls;
        entries.push(json!({
            "block_id": shape.block_id,
            "name": block_name(shape),
            "shapes": shapes,
            "passes": passes,
            "draw_calls": calls,
        }));
    }
    json!({"threshold": threshold, "matched": total > threshold, "total": total, "entries": entries})
}

fn find_uvs(nif: &NifFile, options: &Value) -> Result<Value, String> {
    let u_min = if options.get("u_min").is_some() {
        optional_number(options, "u_min")?
    } else {
        Some(0.0)
    };
    let u_max = optional_number(options, "u_max")?;
    let v_min = if options.get("v_min").is_some() {
        optional_number(options, "v_min")?
    } else {
        Some(0.0)
    };
    let v_max = optional_number(options, "v_max")?;
    if u_min.is_none() && u_max.is_none() && v_min.is_none() && v_max.is_none() {
        return Err("At least one UV bound is required".to_string());
    }
    let mut entries = Vec::new();
    for block in &nif.blocks {
        let uvs = texture_coordinates(block);
        if uvs.is_empty() {
            continue;
        }
        let actual_u_min = uvs.iter().map(|uv| uv[0]).fold(f64::INFINITY, f64::min);
        let actual_u_max = uvs.iter().map(|uv| uv[0]).fold(f64::NEG_INFINITY, f64::max);
        let actual_v_min = uvs.iter().map(|uv| uv[1]).fold(f64::INFINITY, f64::min);
        let actual_v_max = uvs.iter().map(|uv| uv[1]).fold(f64::NEG_INFINITY, f64::max);
        if u_min.is_some_and(|bound| actual_u_min < bound)
            || u_max.is_some_and(|bound| actual_u_max > bound)
            || v_min.is_some_and(|bound| actual_v_min < bound)
            || v_max.is_some_and(|bound| actual_v_max > bound)
        {
            entries.push(json!({
                "block_id": block.block_id,
                "block_type": block.type_name,
                "name": block_name(block),
                "u_min": actual_u_min,
                "u_max": actual_u_max,
                "v_min": actual_v_min,
                "v_max": actual_v_max,
            }));
        }
    }
    Ok(json!({"entries": entries}))
}

struct Geometry {
    block_id: usize,
    name: String,
    vertex_count: usize,
    indices: Vec<u32>,
}

fn rendered_geometry(nif: &NifFile) -> Vec<Geometry> {
    let fo4_family = nif.header.version == (20, 2, 0, 7)
        && nif.header.user_version == 12
        && nif.header.bs_version >= 130;
    let mut output = Vec::new();
    for shape in &nif.blocks {
        if matches!(shape.type_name.as_str(), "NiTriShape" | "NiTriStrips") {
            if referenced_block(nif, shape.get_field("Skin Instance")).is_some() {
                continue;
            }
            if let Some(data) = referenced_block(nif, shape.get_field("Data")) {
                output.push(geometry_from_block(data));
            }
        } else if SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape") {
            if fo4_family || referenced_block(nif, shape.get_field("Skin")).is_none() {
                output.push(geometry_from_block(shape));
            }
        } else if shape.type_name == "NiSkinPartition" {
            for (index, partition) in value_array(shape.get_field("Partitions"))
                .iter()
                .enumerate()
            {
                output.push(Geometry {
                    block_id: shape.block_id,
                    name: format!("{} Partitions[{index}]", block_name(shape)),
                    vertex_count: nested_u64(Some(partition), "Num Vertices").unwrap_or(0) as usize,
                    indices: indices_from_geometry_value(partition),
                });
            }
        }
    }
    output
}

fn geometry_from_block(block: &NifBlock) -> Geometry {
    Geometry {
        block_id: block.block_id,
        name: block_name(block),
        vertex_count: value_u64(block.get_field("Num Vertices"))
            .map(|count| count as usize)
            .unwrap_or_else(|| positions(block).len()),
        indices: indices_from_fields(&block.fields),
    }
}

fn vertex_blocks(nif: &NifFile) -> Vec<(&NifBlock, Vec<[f64; 3]>)> {
    nif.blocks
        .iter()
        .filter(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeomData")
                || SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
        })
        .filter_map(|block| {
            let vertices = positions(block);
            (!vertices.is_empty()).then_some((block, vertices))
        })
        .collect()
}

fn positions(block: &NifBlock) -> Vec<[f64; 3]> {
    let values = if !value_array(block.get_field("Vertex Data")).is_empty() {
        value_array(block.get_field("Vertex Data"))
    } else {
        value_array(block.get_field("Vertices"))
    };
    values
        .iter()
        .filter_map(|value| {
            let vertex = nested_value(Some(value), "Vertex").unwrap_or(value);
            vec3(vertex)
        })
        .collect()
}

fn indices_from_fields(fields: &IndexMap<String, NifValue>) -> Vec<u32> {
    fields
        .iter()
        .filter(|(name, _)| matches!(bare_name(name), "Triangles" | "Strips"))
        .flat_map(|(name, value)| {
            if bare_name(name) == "Strips" {
                strip_arrays_to_triangles(value)
            } else {
                indices_from_value(value)
            }
        })
        .collect()
}

fn indices_from_geometry_value(value: &NifValue) -> Vec<u32> {
    let Some(fields) = as_struct(value) else {
        return Vec::new();
    };
    indices_from_fields(fields)
}

fn strip_arrays_to_triangles(value: &NifValue) -> Vec<u32> {
    let strips = match value {
        NifValue::Array(values) => values,
        _ => return Vec::new(),
    };
    strips
        .iter()
        .flat_map(|strip| {
            let points = match strip {
                NifValue::Array(points) => points
                    .iter()
                    .filter_map(|point| value_u64(Some(point)))
                    .filter_map(|point| u32::try_from(point).ok())
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            points
                .windows(3)
                .enumerate()
                .filter_map(|(index, triangle)| {
                    let [a, b, c] = <[u32; 3]>::try_from(triangle).ok()?;
                    if a == b || b == c || a == c {
                        return None;
                    }
                    Some(if index % 2 == 0 { [a, b, c] } else { [b, a, c] })
                })
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect()
}

fn indices_from_value(value: &NifValue) -> Vec<u32> {
    match value {
        NifValue::UInt(value) => u32::try_from(*value).into_iter().collect(),
        NifValue::Int(value) => u32::try_from(*value).into_iter().collect(),
        NifValue::Array(values) => values.iter().flat_map(indices_from_value).collect(),
        NifValue::Struct(fields) => fields
            .iter()
            .filter(|(name, _)| {
                matches!(bare_name(name), "v1" | "v2" | "v3" | "Triangles" | "Strips")
            })
            .flat_map(|(_, value)| indices_from_value(value))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_empty_node(nif: &NifFile, block: &NifBlock) -> bool {
    referenced_block(nif, block.get_field("Collision Object")).is_none()
        && ref_values(block.get_field("Children"))
            .into_iter()
            .all(|id| {
                nif.get_block(id as usize)
                    .is_none_or(|child| !SCHEMA.is_subtype_of(&child.type_name, "NiAVObject"))
            })
}

fn collect_descendants(nif: &NifFile, root: usize, output: &mut HashSet<usize>) {
    if !output.insert(root) {
        return;
    }
    let Some(block) = nif.get_block(root) else {
        return;
    };
    for child in ref_values(block.get_field("Children")) {
        collect_descendants(nif, child as usize, output);
    }
}

fn geometry_shader<'a>(nif: &'a NifFile, shape: &NifBlock) -> Option<&'a NifBlock> {
    referenced_block(nif, shape.get_field("Shader Property")).or_else(|| {
        ref_values(shape.get_field("Properties"))
            .into_iter()
            .filter_map(|id| nif.get_block(id as usize))
            .find(|property| SCHEMA.is_subtype_of(&property.type_name, "BSShaderProperty"))
    })
}

fn rigid_body_is_dynamic(nif: &NifFile, body: &NifBlock) -> bool {
    let motion = value_u64(rigid_body_value(body, "Motion System")).unwrap_or(0);
    let layer = rigid_body_nested_u64(body, "Havok Filter", "Layer").unwrap_or(0);
    if matches!(motion, 0 | 7) || (motion == 6 && layer != 8) {
        return false;
    }
    if nif.header.version == (20, 2, 0, 7)
        && nif.header.user_version == 12
        && nif.header.bs_version >= 83
    {
        let quality = value_u64(rigid_body_value(body, "Motion Quality")).unwrap_or(0);
        return layer > 2 && !matches!(quality, 0 | 1);
    }
    true
}

fn rigid_body_value<'a>(block: &'a NifBlock, name: &str) -> Option<&'a NifValue> {
    block
        .get_field(name)
        .or_else(|| nested_value(block.get_field("Rigid Body Info"), name))
}

fn rigid_body_nested_u64(block: &NifBlock, group: &str, name: &str) -> Option<u64> {
    nested_u64(block.get_field(group), name).or_else(|| {
        nested_value(block.get_field("Rigid Body Info"), group)
            .and_then(|value| nested_u64(Some(value), name))
    })
}

fn referenced_block<'a>(nif: &'a NifFile, value: Option<&NifValue>) -> Option<&'a NifBlock> {
    value_ref(value)
        .filter(|id| *id >= 0)
        .and_then(|id| nif.get_block(id as usize))
}

fn block_name(block: &NifBlock) -> String {
    block
        .get_field("Name")
        .and_then(value_string)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} {}", block.block_id, block.type_name))
}

fn texture_coordinates(block: &NifBlock) -> Vec<[f64; 2]> {
    if SCHEMA.is_subtype_of(&block.type_name, "BSTriShape") || block.type_name == "NiSkinPartition"
    {
        let flags = value_u64(block.get_field("Vertex Desc")).unwrap_or(0) >> 44;
        if flags & 0x0002 == 0 {
            return Vec::new();
        }
        return value_array(block.get_field("Vertex Data"))
            .iter()
            .filter_map(|vertex| nested_value(Some(vertex), "UV"))
            .filter_map(tex_coord)
            .collect();
    }
    if SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeomData") {
        return value_array(block.get_field("UV Sets"))
            .first()
            .map(|set| value_array(Some(set)))
            .unwrap_or_default()
            .iter()
            .filter_map(tex_coord)
            .collect();
    }
    Vec::new()
}

fn tex_coord(value: &NifValue) -> Option<[f64; 2]> {
    let fields = as_struct(value)?;
    Some([
        named_value(fields, "u").and_then(|value| value_f64(Some(value)))?,
        named_value(fields, "v").and_then(|value| value_f64(Some(value)))?,
    ])
}

fn vec3(value: &NifValue) -> Option<[f64; 3]> {
    match value {
        NifValue::Vec3(values) | NifValue::Color3(values) => Some(values.map(|value| value as f64)),
        NifValue::Struct(fields) => Some([
            named_value(fields, "x").and_then(|value| value_f64(Some(value)))?,
            named_value(fields, "y").and_then(|value| value_f64(Some(value)))?,
            named_value(fields, "z").and_then(|value| value_f64(Some(value)))?,
        ]),
        _ => None,
    }
}

fn value_is_zero_vector(value: &NifValue) -> bool {
    vec3(value).is_some_and(|vector| vector.iter().all(|value| *value == 0.0))
}

fn value_is_identity_matrix(value: &NifValue) -> bool {
    match value {
        NifValue::Matrix33(matrix) => {
            *matrix == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        }
        NifValue::Quaternion(quaternion) | NifValue::Vec4(quaternion) => {
            *quaternion == [0.0, 0.0, 0.0, 1.0] || *quaternion == [1.0, 0.0, 0.0, 0.0]
        }
        _ => false,
    }
}

fn nif_value_json(value: &NifValue) -> Value {
    match value {
        NifValue::Null => Value::Null,
        NifValue::Bool(value) => json!(value),
        NifValue::Int(value) => json!(value),
        NifValue::UInt(value) => json!(value),
        NifValue::Float(value) => json!(value),
        NifValue::FloatNan(value) => json!(value),
        NifValue::String(value) | NifValue::Char(value) => json!(value),
        NifValue::Ref(value) => json!(value),
        NifValue::Vec3(value) | NifValue::Color3(value) => json!(value),
        NifValue::Vec4(value) | NifValue::Color4(value) | NifValue::Quaternion(value) => {
            json!(value)
        }
        NifValue::Matrix33(value) => json!(value),
        NifValue::Matrix44(value) => json!(value),
        NifValue::Array(values) => Value::Array(values.iter().map(nif_value_json).collect()),
        NifValue::Struct(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), nif_value_json(value)))
                .collect(),
        ),
        NifValue::Bytes(values) => json!(values),
    }
}

fn as_struct(value: &NifValue) -> Option<&IndexMap<String, NifValue>> {
    match value {
        NifValue::Struct(fields) => Some(fields),
        _ => None,
    }
}

fn named_value<'a>(fields: &'a IndexMap<String, NifValue>, name: &str) -> Option<&'a NifValue> {
    fields.get(name).or_else(|| {
        fields
            .iter()
            .find(|(field, _)| bare_name(field).eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

fn nested_value<'a>(value: Option<&'a NifValue>, name: &str) -> Option<&'a NifValue> {
    named_value(as_struct(value?)?, name)
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

fn ref_values(value: Option<&NifValue>) -> Vec<i32> {
    value_array(value)
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .filter(|value| *value >= 0)
        .collect()
}

fn value_string(value: &NifValue) -> Option<&str> {
    match value {
        NifValue::String(value) | NifValue::Char(value) => Some(value),
        _ => None,
    }
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

fn option_bool(options: &Value, name: &str) -> Option<bool> {
    options.get(name).and_then(Value::as_bool)
}

fn option_u64(options: &Value, name: &str) -> Option<u64> {
    options.get(name).and_then(Value::as_u64)
}

fn option_f64(options: &Value, name: &str) -> Option<f64> {
    options.get(name).and_then(Value::as_f64)
}

fn optional_number(options: &Value, name: &str) -> Result<Option<f64>, String> {
    match options.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| format!("{name} must be a finite number")),
    }
}

fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_report_uses_nif_default_minimums_and_requires_an_explicit_bound() {
        assert!(find_uvs(&NifFile::new("fo4"), &json!({})).is_ok());
        assert_eq!(
            find_uvs(
                &NifFile::new("fo4"),
                &json!({"u_min": null, "u_max": null, "v_min": null, "v_max": null})
            )
            .unwrap_err(),
            "At least one UV bound is required"
        );
    }

    #[test]
    fn transform_report_handles_oblivion_header() {
        let mut nif = NifFile::new("oblivion");
        nif.blocks[0].set_field("Translation", NifValue::Vec3([1.0, 2.0, 3.0]));
        let report = transform_information(&nif, &json!({"skip_empty": false}));
        assert_eq!(report["entries"].as_array().unwrap().len(), 1);
        assert_eq!(crate::validation::nif_game_label(&nif), "oblivion");
    }

    #[test]
    fn fo3_draw_call_report_reads_shader_flags() {
        let mut nif = NifFile::new("fnv");
        let shader = nif.add_block(
            "BSShaderPPLightingProperty",
            Some(IndexMap::from([(
                "Shader Flags".to_string(),
                NifValue::UInt((1 << 7) | (1 << 10)),
            )])),
        );
        nif.add_block(
            "NiTriShape",
            Some(IndexMap::from([(
                "Properties".to_string(),
                NifValue::Array(vec![NifValue::Ref(shader as i32)]),
            )])),
        );

        let report = find_excessive_draw_calls(&nif, &json!({}));
        assert_eq!(report["total"], 3);
        assert_eq!(report["entries"][0]["passes"], 3);
    }

    #[test]
    fn uv_report_only_reads_nif_geometry_uv_sources() {
        let mut nif = NifFile::new("fo4");
        nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                ("Vertex Desc".to_string(), NifValue::UInt(0)),
                (
                    "Vertex Data".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                        "UV".to_string(),
                        NifValue::Struct(IndexMap::from([
                            ("u".to_string(), NifValue::Float(-2.0)),
                            ("v".to_string(), NifValue::Float(-2.0)),
                        ])),
                    )]))]),
                ),
            ])),
        );
        nif.add_block(
            "NiStringExtraData",
            Some(IndexMap::from([(
                "Unrelated".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("u".to_string(), NifValue::Float(-3.0)),
                    ("v".to_string(), NifValue::Float(-3.0)),
                ])),
            )])),
        );

        let report = find_uvs(&nif, &json!({})).unwrap();
        assert!(report["entries"].as_array().unwrap().is_empty());

        nif.blocks[1].set_field("Vertex Desc", NifValue::UInt(0x0002 << 44));
        let report = find_uvs(&nif, &json!({})).unwrap();
        assert_eq!(report["entries"].as_array().unwrap().len(), 1);
        assert_eq!(report["entries"][0]["block_id"], 1);
    }
}
