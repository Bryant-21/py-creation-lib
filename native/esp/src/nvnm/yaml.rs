use super::parser::NvnmError;
use super::types::{
    NvnmCoverEntry, NvnmCoverTriangleMapping, NvnmDoorRef, NvnmEdgeLink, NvnmGrid, NvnmGridCell,
    NvnmParent, NvnmPayload, NvnmTriangle, NvnmVertex, NvnmWaypoint,
};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Encode helpers
// ---------------------------------------------------------------------------

fn hex_u16_flags(value: u16) -> String {
    format!("0x{value:04X}")
}

fn hex_u32_flags(value: u32) -> String {
    format!("0x{value:08X}")
}

fn hex_form_id(value: u32) -> String {
    format!("{value:08X}")
}

fn parent_to_json(parent: &NvnmParent) -> Value {
    let mut map = Map::new();
    match parent {
        NvnmParent::Interior { cell } => {
            map.insert("interior_cell".into(), Value::String(hex_form_id(*cell)));
        }
        NvnmParent::Exterior {
            world,
            grid_x,
            grid_y,
        } => {
            map.insert("exterior_world".into(), Value::String(hex_form_id(*world)));
            map.insert("cell".into(), json!([*grid_x, *grid_y]));
        }
    }
    Value::Object(map)
}

fn vertex_to_json(v: &NvnmVertex) -> Value {
    json!([v.x, v.y, v.z])
}

fn triangle_to_json(t: &NvnmTriangle) -> Value {
    json!({
        "v": [t.vertices[0], t.vertices[1], t.vertices[2]],
        "links": [t.links[0], t.links[1], t.links[2]],
        "flags": hex_u16_flags(t.flags),
        "cover_marker_hex": hex::encode_upper(t.cover_marker),
    })
}

fn edge_link_to_json(e: &NvnmEdgeLink) -> Value {
    json!({ "row_hex": hex::encode_upper(e.row) })
}

fn door_ref_to_json(d: &NvnmDoorRef) -> Value {
    json!({
        "triangle": d.triangle_index,
        "padding_hex": hex::encode_upper(d.padding),
        "door_ref": hex_form_id(d.door_ref_form_id),
    })
}

fn cover_to_json(c: &NvnmCoverEntry) -> Value {
    json!({
        "vertex_1": c.vertex_1,
        "vertex_2": c.vertex_2,
        "data_byte_1": c.data_byte_1,
        "data_byte_2": c.data_byte_2,
        "data_byte_3": c.data_byte_3,
        "data_byte_4": c.data_byte_4,
    })
}

fn cover_mapping_to_json(m: &NvnmCoverTriangleMapping) -> Value {
    json!({ "cover": m.cover, "triangle": m.triangle })
}

fn waypoint_to_json(w: &NvnmWaypoint) -> Value {
    json!({
        "x": w.x,
        "y": w.y,
        "z": w.z,
        "triangle": w.triangle,
        "flags": hex_u32_flags(w.flags),
    })
}

fn grid_cell_to_json(cell: &NvnmGridCell) -> Value {
    Value::Array(cell.triangle_indices.iter().map(|v| json!(*v)).collect())
}

fn grid_to_json(grid: &NvnmGrid) -> Value {
    let mut map = Map::new();
    map.insert("divisor".into(), json!(grid.divisor));
    if grid.divisor > 0 {
        map.insert("grid_size_x".into(), json!(grid.grid_size_x));
        map.insert("grid_size_y".into(), json!(grid.grid_size_y));
        map.insert("bounds_min_x".into(), json!(grid.bounds_min_x));
        map.insert("bounds_min_y".into(), json!(grid.bounds_min_y));
        map.insert("bounds_min_z".into(), json!(grid.bounds_min_z));
        map.insert("bounds_max_x".into(), json!(grid.bounds_max_x));
        map.insert("bounds_max_y".into(), json!(grid.bounds_max_y));
        map.insert("bounds_max_z".into(), json!(grid.bounds_max_z));
        let cells: Vec<Value> = grid.cells.iter().map(grid_cell_to_json).collect();
        map.insert("cells".into(), Value::Array(cells));
    }
    Value::Object(map)
}

pub fn nvnm_to_yaml(payload: &NvnmPayload) -> Value {
    let mut map = Map::new();
    map.insert("version".into(), json!(payload.version));
    map.insert("flags".into(), Value::String(hex_u32_flags(payload.flags)));
    map.insert("parent".into(), parent_to_json(&payload.parent));
    map.insert(
        "vertices".into(),
        Value::Array(payload.vertices.iter().map(vertex_to_json).collect()),
    );
    map.insert(
        "triangles".into(),
        Value::Array(payload.triangles.iter().map(triangle_to_json).collect()),
    );
    map.insert(
        "edge_links".into(),
        Value::Array(payload.edge_links.iter().map(edge_link_to_json).collect()),
    );
    map.insert(
        "door_refs".into(),
        Value::Array(payload.door_refs.iter().map(door_ref_to_json).collect()),
    );
    map.insert(
        "cover_array".into(),
        Value::Array(payload.cover_array.iter().map(cover_to_json).collect()),
    );
    map.insert(
        "cover_triangle_mappings".into(),
        Value::Array(
            payload
                .cover_triangle_mappings
                .iter()
                .map(cover_mapping_to_json)
                .collect(),
        ),
    );
    map.insert(
        "waypoints".into(),
        Value::Array(payload.waypoints.iter().map(waypoint_to_json).collect()),
    );
    map.insert("grid".into(), grid_to_json(&payload.grid));
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Decode helpers
// ---------------------------------------------------------------------------

fn err(message: impl Into<String>) -> NvnmError {
    NvnmError::Other(message.into())
}

fn get<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a Value, NvnmError> {
    map.get(key)
        .ok_or_else(|| err(format!("missing field: {key}")))
}

fn as_object<'a>(value: &'a Value, key: &str) -> Result<&'a Map<String, Value>, NvnmError> {
    value
        .as_object()
        .ok_or_else(|| err(format!("expected object for {key}")))
}

fn as_array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, NvnmError> {
    value
        .as_array()
        .ok_or_else(|| err(format!("expected array for {key}")))
}

fn parse_u32_from_value(value: &Value, key: &str) -> Result<u32, NvnmError> {
    if let Some(n) = value.as_u64() {
        u32::try_from(n).map_err(|_| err(format!("{key} out of u32 range")))
    } else if let Some(s) = value.as_str() {
        let stripped = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        u32::from_str_radix(stripped, 16)
            .map_err(|e| err(format!("invalid hex u32 for {key}: {e}")))
    } else {
        Err(err(format!("{key} must be integer or hex string")))
    }
}

fn parse_u16_from_value(value: &Value, key: &str) -> Result<u16, NvnmError> {
    if let Some(n) = value.as_u64() {
        u16::try_from(n).map_err(|_| err(format!("{key} out of u16 range")))
    } else if let Some(s) = value.as_str() {
        let stripped = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        u16::from_str_radix(stripped, 16)
            .map_err(|e| err(format!("invalid hex u16 for {key}: {e}")))
    } else {
        Err(err(format!("{key} must be integer or hex string")))
    }
}

fn parse_form_id_from_value(value: &Value, key: &str) -> Result<u32, NvnmError> {
    let s = value
        .as_str()
        .ok_or_else(|| err(format!("{key} must be hex string")))?;
    let stripped = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(stripped, 16)
        .map_err(|e| err(format!("invalid form_id hex for {key}: {e}")))
}

fn parse_i16(value: &Value, key: &str) -> Result<i16, NvnmError> {
    value
        .as_i64()
        .and_then(|v| i16::try_from(v).ok())
        .ok_or_else(|| err(format!("{key} must be i16")))
}

fn parse_u16_int(value: &Value, key: &str) -> Result<u16, NvnmError> {
    value
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
        .ok_or_else(|| err(format!("{key} must be u16")))
}

fn parse_u8(value: &Value, key: &str) -> Result<u8, NvnmError> {
    value
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
        .ok_or_else(|| err(format!("{key} must be u8")))
}

fn parse_f32(value: &Value, key: &str) -> Result<f32, NvnmError> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| err(format!("{key} must be float")))
}

fn parse_hex_bytes_fixed<const N: usize>(value: &Value, key: &str) -> Result<[u8; N], NvnmError> {
    let s = value
        .as_str()
        .ok_or_else(|| err(format!("{key} must be hex string")))?;
    let bytes = hex::decode(s.trim()).map_err(|e| err(format!("invalid hex for {key}: {e}")))?;
    if bytes.len() != N {
        return Err(err(format!(
            "{key} must be {} bytes ({} hex chars); got {}",
            N,
            N * 2,
            bytes.len()
        )));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn parse_parent(value: &Value) -> Result<NvnmParent, NvnmError> {
    let obj = as_object(value, "parent")?;
    if let Some(cell_value) = obj.get("interior_cell") {
        let cell = parse_form_id_from_value(cell_value, "parent.interior_cell")?;
        return Ok(NvnmParent::Interior { cell });
    }
    let world_value = obj
        .get("exterior_world")
        .ok_or_else(|| err("parent must have interior_cell or exterior_world"))?;
    let world = parse_form_id_from_value(world_value, "parent.exterior_world")?;
    let cell_arr = as_array(get(obj, "cell")?, "parent.cell")?;
    if cell_arr.len() != 2 {
        return Err(err("parent.cell must be [grid_x, grid_y]"));
    }
    let grid_x = parse_i16(&cell_arr[0], "parent.cell[0]")?;
    let grid_y = parse_i16(&cell_arr[1], "parent.cell[1]")?;
    Ok(NvnmParent::Exterior {
        world,
        grid_x,
        grid_y,
    })
}

fn parse_vertex(value: &Value) -> Result<NvnmVertex, NvnmError> {
    let arr = as_array(value, "vertex")?;
    if arr.len() != 3 {
        return Err(err("vertex must be [x, y, z]"));
    }
    Ok(NvnmVertex {
        x: parse_f32(&arr[0], "vertex.x")?,
        y: parse_f32(&arr[1], "vertex.y")?,
        z: parse_f32(&arr[2], "vertex.z")?,
    })
}

fn parse_triangle(value: &Value) -> Result<NvnmTriangle, NvnmError> {
    let obj = as_object(value, "triangle")?;
    let v_arr = as_array(get(obj, "v")?, "triangle.v")?;
    if v_arr.len() != 3 {
        return Err(err("triangle.v must be [v0, v1, v2]"));
    }
    let links_arr = as_array(get(obj, "links")?, "triangle.links")?;
    if links_arr.len() != 3 {
        return Err(err("triangle.links must be [l0, l1, l2]"));
    }
    let flags = parse_u16_from_value(get(obj, "flags")?, "triangle.flags")?;
    let cover_marker =
        parse_hex_bytes_fixed::<9>(get(obj, "cover_marker_hex")?, "triangle.cover_marker_hex")?;
    Ok(NvnmTriangle {
        vertices: [
            parse_u16_int(&v_arr[0], "triangle.v[0]")?,
            parse_u16_int(&v_arr[1], "triangle.v[1]")?,
            parse_u16_int(&v_arr[2], "triangle.v[2]")?,
        ],
        links: [
            parse_i16(&links_arr[0], "triangle.links[0]")?,
            parse_i16(&links_arr[1], "triangle.links[1]")?,
            parse_i16(&links_arr[2], "triangle.links[2]")?,
        ],
        cover_marker,
        flags,
    })
}

fn parse_edge_link(value: &Value) -> Result<NvnmEdgeLink, NvnmError> {
    let obj = as_object(value, "edge_link")?;
    let row = parse_hex_bytes_fixed::<11>(get(obj, "row_hex")?, "edge_link.row_hex")?;
    Ok(NvnmEdgeLink { row })
}

fn parse_door_ref(value: &Value) -> Result<NvnmDoorRef, NvnmError> {
    let obj = as_object(value, "door_ref")?;
    let triangle_index = parse_i16(get(obj, "triangle")?, "door_ref.triangle")?;
    let padding = parse_hex_bytes_fixed::<4>(get(obj, "padding_hex")?, "door_ref.padding_hex")?;
    let door_ref_form_id = parse_form_id_from_value(get(obj, "door_ref")?, "door_ref.door_ref")?;
    Ok(NvnmDoorRef {
        triangle_index,
        padding,
        door_ref_form_id,
    })
}

fn parse_cover(value: &Value) -> Result<NvnmCoverEntry, NvnmError> {
    let obj = as_object(value, "cover")?;
    Ok(NvnmCoverEntry {
        vertex_1: parse_u16_int(get(obj, "vertex_1")?, "cover.vertex_1")?,
        vertex_2: parse_u16_int(get(obj, "vertex_2")?, "cover.vertex_2")?,
        data_byte_1: parse_u8(get(obj, "data_byte_1")?, "cover.data_byte_1")?,
        data_byte_2: parse_u8(get(obj, "data_byte_2")?, "cover.data_byte_2")?,
        data_byte_3: parse_u8(get(obj, "data_byte_3")?, "cover.data_byte_3")?,
        data_byte_4: parse_u8(get(obj, "data_byte_4")?, "cover.data_byte_4")?,
    })
}

fn parse_cover_mapping(value: &Value) -> Result<NvnmCoverTriangleMapping, NvnmError> {
    let obj = as_object(value, "cover_triangle_mapping")?;
    Ok(NvnmCoverTriangleMapping {
        cover: parse_u16_int(get(obj, "cover")?, "cover_triangle_mapping.cover")?,
        triangle: parse_i16(get(obj, "triangle")?, "cover_triangle_mapping.triangle")?,
    })
}

fn parse_waypoint(value: &Value) -> Result<NvnmWaypoint, NvnmError> {
    let obj = as_object(value, "waypoint")?;
    Ok(NvnmWaypoint {
        x: parse_f32(get(obj, "x")?, "waypoint.x")?,
        y: parse_f32(get(obj, "y")?, "waypoint.y")?,
        z: parse_f32(get(obj, "z")?, "waypoint.z")?,
        triangle: parse_i16(get(obj, "triangle")?, "waypoint.triangle")?,
        flags: parse_u32_from_value(get(obj, "flags")?, "waypoint.flags")?,
    })
}

fn parse_grid_cell(value: &Value) -> Result<NvnmGridCell, NvnmError> {
    let arr = as_array(value, "grid.cell")?;
    let mut triangle_indices = Vec::with_capacity(arr.len());
    for (i, entry) in arr.iter().enumerate() {
        triangle_indices.push(parse_i16(entry, &format!("grid.cell[{i}]"))?);
    }
    Ok(NvnmGridCell { triangle_indices })
}

fn parse_grid(value: &Value) -> Result<NvnmGrid, NvnmError> {
    let obj = as_object(value, "grid")?;
    let divisor = parse_u32_from_value(get(obj, "divisor")?, "grid.divisor")?;
    if divisor == 0 {
        return Ok(NvnmGrid::default());
    }
    let grid_size_x = parse_f32(get(obj, "grid_size_x")?, "grid.grid_size_x")?;
    let grid_size_y = parse_f32(get(obj, "grid_size_y")?, "grid.grid_size_y")?;
    let bounds_min_x = parse_f32(get(obj, "bounds_min_x")?, "grid.bounds_min_x")?;
    let bounds_min_y = parse_f32(get(obj, "bounds_min_y")?, "grid.bounds_min_y")?;
    let bounds_min_z = parse_f32(get(obj, "bounds_min_z")?, "grid.bounds_min_z")?;
    let bounds_max_x = parse_f32(get(obj, "bounds_max_x")?, "grid.bounds_max_x")?;
    let bounds_max_y = parse_f32(get(obj, "bounds_max_y")?, "grid.bounds_max_y")?;
    let bounds_max_z = parse_f32(get(obj, "bounds_max_z")?, "grid.bounds_max_z")?;
    let cells_arr = as_array(get(obj, "cells")?, "grid.cells")?;
    let expected = (divisor as usize)
        .checked_mul(divisor as usize)
        .ok_or_else(|| err(format!("grid.divisor² overflow ({divisor})")))?;
    if cells_arr.len() != expected {
        return Err(err(format!(
            "grid.cells length {} does not match divisor² ({})",
            cells_arr.len(),
            expected
        )));
    }
    let mut cells = Vec::with_capacity(expected);
    for entry in cells_arr {
        cells.push(parse_grid_cell(entry)?);
    }
    Ok(NvnmGrid {
        divisor,
        grid_size_x,
        grid_size_y,
        bounds_min_x,
        bounds_min_y,
        bounds_min_z,
        bounds_max_x,
        bounds_max_y,
        bounds_max_z,
        cells,
    })
}

pub fn nvnm_from_yaml(value: &Value) -> Result<NvnmPayload, NvnmError> {
    let obj = as_object(value, "nvnm")?;
    let version = parse_u32_from_value(get(obj, "version")?, "version")?;
    let flags = parse_u32_from_value(get(obj, "flags")?, "flags")?;
    let parent = parse_parent(get(obj, "parent")?)?;
    let vertices = as_array(get(obj, "vertices")?, "vertices")?
        .iter()
        .map(parse_vertex)
        .collect::<Result<Vec<_>, _>>()?;
    let triangles = as_array(get(obj, "triangles")?, "triangles")?
        .iter()
        .map(parse_triangle)
        .collect::<Result<Vec<_>, _>>()?;
    let edge_links = as_array(get(obj, "edge_links")?, "edge_links")?
        .iter()
        .map(parse_edge_link)
        .collect::<Result<Vec<_>, _>>()?;
    let door_refs = as_array(get(obj, "door_refs")?, "door_refs")?
        .iter()
        .map(parse_door_ref)
        .collect::<Result<Vec<_>, _>>()?;
    let cover_array = as_array(get(obj, "cover_array")?, "cover_array")?
        .iter()
        .map(parse_cover)
        .collect::<Result<Vec<_>, _>>()?;
    let cover_triangle_mappings = as_array(
        get(obj, "cover_triangle_mappings")?,
        "cover_triangle_mappings",
    )?
    .iter()
    .map(parse_cover_mapping)
    .collect::<Result<Vec<_>, _>>()?;
    let waypoints = as_array(get(obj, "waypoints")?, "waypoints")?
        .iter()
        .map(parse_waypoint)
        .collect::<Result<Vec<_>, _>>()?;
    let grid = parse_grid(get(obj, "grid")?)?;
    Ok(NvnmPayload {
        version,
        flags,
        parent,
        vertices,
        triangles,
        edge_links,
        door_refs,
        cover_array,
        cover_triangle_mappings,
        waypoints,
        grid,
    })
}
