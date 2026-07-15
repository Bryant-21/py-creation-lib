use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Map, Number, Value};

use crate::bsrefl::Chunk;
use crate::cdb::{CdbPayload, ClassDefPayload, ComponentBlobPayload, MaterialObjectPayload};
use crate::string_table::STRING_TABLE;

const ST_STRING: u32 = 1;
const ST_LIST: u32 = 2;
const ST_MAP: u32 = 3;
const ST_REF: u32 = 4;
const ST_INT8: u32 = 7;
const ST_UINT8: u32 = 8;
const ST_INT16: u32 = 9;
const ST_UINT16: u32 = 10;
const ST_INT32: u32 = 11;
const ST_UINT32: u32 = 12;
const ST_INT64: u32 = 13;
const ST_UINT64: u32 = 14;
const ST_BOOL: u32 = 15;
const ST_FLOAT: u32 = 16;
const ST_DOUBLE: u32 = 17;
const ST_UNKNOWN: u32 = 18;
const ST_BSCOMPONENTDB2_ID: u32 = 153;

#[derive(Debug, Clone, Serialize)]
pub struct Ce2MaterialPayload {
    pub name: String,
    pub object_path: String,
    pub layers: Vec<Ce2LayerPayload>,
    pub blenders: Vec<Ce2BlenderPayload>,
    pub lod_materials: Vec<Ce2MaterialPayload>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2LayerPayload {
    pub material: Ce2MaterialPropsPayload,
    pub texture_set: Ce2TextureSetPayload,
    pub uv_stream: Ce2UvStreamPayload,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2TextureSetPayload {
    pub diffuse: String,
    pub normal: String,
    pub opacity: String,
    pub rough: String,
    pub metal: String,
    pub ao: String,
    pub emissive: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2UvStreamPayload {
    pub scale_u: f32,
    pub scale_v: f32,
    pub offset_u: f32,
    pub offset_v: f32,
    pub channel: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2MaterialPropsPayload {
    pub smoothness: f32,
    pub metalness: f32,
    pub emissive_color: [f32; 3],
    pub emissive_multiplier: f32,
    pub alpha: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2BlenderPayload {
    pub mode: i32,
    pub mask_texture: String,
}

type ClassDefMap<'a> = HashMap<&'a str, &'a ClassDefPayload>;

pub fn project(cdb: &CdbPayload, root_db_id: u32) -> Option<Ce2MaterialPayload> {
    let root = cdb.objects.iter().find(|obj| obj.db_id == root_db_id)?;
    let class_defs = class_def_map(&cdb.class_defs);
    let mut children_by_parent = HashMap::<u32, Vec<&MaterialObjectPayload>>::new();
    for obj in &cdb.objects {
        if let Some(parent_db_id) = obj.parent_db_id {
            children_by_parent
                .entry(parent_db_id)
                .or_default()
                .push(obj);
        }
    }

    let mut material = default_material_payload();
    let mut state = WalkState::default();
    walk_object(
        root,
        &children_by_parent,
        &class_defs,
        &mut material,
        &mut state,
    );
    Some(material)
}

pub fn walk_component(
    component: &ComponentBlobPayload,
    class_defs: &[ClassDefPayload],
) -> Option<Value> {
    let class_defs = class_def_map(class_defs);
    walk_component_map(component, &class_defs).map(Value::Object)
}

#[derive(Default)]
struct WalkState {
    current_layer: Option<usize>,
    mr_texture_slot: usize,
}

fn walk_object(
    obj: &MaterialObjectPayload,
    children_by_parent: &HashMap<u32, Vec<&MaterialObjectPayload>>,
    class_defs: &ClassDefMap<'_>,
    material: &mut Ce2MaterialPayload,
    state: &mut WalkState,
) {
    for component in &obj.components {
        dispatch_component(component, class_defs, material, state);
    }
    if let Some(children) = children_by_parent.get(&obj.db_id) {
        for child in children {
            walk_object(child, children_by_parent, class_defs, material, state);
        }
    }
}

fn dispatch_component(
    component: &ComponentBlobPayload,
    class_defs: &ClassDefMap<'_>,
    material: &mut Ce2MaterialPayload,
    state: &mut WalkState,
) {
    let data = walk_component_map(component, class_defs).unwrap_or_default();
    match component.class_name.as_str() {
        "BSMaterial::LayerID" => {
            material.layers.push(default_layer_payload());
            state.current_layer = material.layers.len().checked_sub(1);
            state.mr_texture_slot = 0;
        }
        "BSMaterial::TextureSetID" => {
            state.mr_texture_slot = 0;
        }
        "BSMaterial::MaterialID" => {
            if let Some(layer) = current_layer_mut(material, state) {
                populate_material_props(&mut layer.material, &data);
            }
        }
        "BSMaterial::BlenderID" => {
            let mut blender = default_blender_payload();
            if let Some(mode) = get_i32(&data, "eMode") {
                blender.mode = mode;
            }
            material.blenders.push(blender);
        }
        "BSMaterial::UVStreamID" => {
            if let Some(layer) = current_layer_mut(material, state) {
                if let Some(value) = get_f32(&data, "fScaleU") {
                    layer.uv_stream.scale_u = value;
                }
                if let Some(value) = get_f32(&data, "fScaleV") {
                    layer.uv_stream.scale_v = value;
                }
                if let Some(value) = get_f32(&data, "fOffsetU") {
                    layer.uv_stream.offset_u = value;
                }
                if let Some(value) = get_f32(&data, "fOffsetV") {
                    layer.uv_stream.offset_v = value;
                }
                if let Some(value) = get_i32(&data, "iChannel") {
                    layer.uv_stream.channel = value;
                }
            }
        }
        "BSMaterial::LODMaterialID" => {
            let mut lod_material = default_material_payload();
            lod_material.name = format!("lod_{}", component_index(component));
            material.lod_materials.push(lod_material);
        }
        "BSMaterial::MRTextureFile" => {
            let Some(layer_index) = state.current_layer else {
                return;
            };
            let Some(filename) = get_string(&data, "FileName") else {
                return;
            };
            if filename.is_empty() {
                return;
            }
            let texture_set = &mut material.layers[layer_index].texture_set;
            match state.mr_texture_slot {
                0 => texture_set.diffuse = filename,
                1 => texture_set.normal = filename,
                2 => texture_set.rough = filename,
                3 => texture_set.metal = filename,
                4 => texture_set.ao = filename,
                5 => texture_set.opacity = filename,
                6 => texture_set.emissive = filename,
                _ => {}
            }
            state.mr_texture_slot += 1;
        }
        "BSMaterial::Scale" => {
            if let Some(layer) = current_layer_mut(material, state) {
                if let Some(value) = get_f32(&data, "fScaleU") {
                    layer.uv_stream.scale_u = value;
                }
                if let Some(value) = get_f32(&data, "fScaleV") {
                    layer.uv_stream.scale_v = value;
                }
            }
        }
        "BSMaterial::AlphaSettingsComponent" => {
            if let Some(layer) = current_layer_mut(material, state) {
                if let Some(value) = get_f32(&data, "fAlpha") {
                    layer.material.alpha = value;
                }
            }
        }
        "BSMaterial::EmissiveSettingsComponent" | "BSMaterial::LayeredEmissivityComponent" => {
            if let Some(layer) = current_layer_mut(material, state) {
                if let Some(color) = get_color(&data, "Color") {
                    layer.material.emissive_color = color;
                }
                if let Some(value) = get_f32(&data, "fMultiplier") {
                    layer.material.emissive_multiplier = value;
                }
            }
        }
        "BSComponentDB::CTName" => {
            let Some(name) = get_string(&data, "Name") else {
                return;
            };
            if name.is_empty() {
                return;
            }
            if material.object_path.is_empty() {
                material.object_path = name.clone();
            }
            if let Some(layer) = current_layer_mut(material, state) {
                layer.name = name;
            }
        }
        _ => {}
    }
}

fn walk_component_map(
    component: &ComponentBlobPayload,
    class_defs: &ClassDefMap<'_>,
) -> Option<Map<String, Value>> {
    let cdef = class_defs.get(component.class_name.as_str())?;
    let mut chunk = Chunk::new(&component.body);
    walk_fields(&mut chunk, cdef, component.is_diff, class_defs)
}

fn walk_fields(
    chunk: &mut Chunk<'_>,
    cdef: &ClassDefPayload,
    is_diff: bool,
    class_defs: &ClassDefMap<'_>,
) -> Option<Map<String, Value>> {
    if cdef.field_count == 0 {
        return Some(Map::new());
    }

    let mut result = Map::new();
    let n_max = cdef.field_count.saturating_sub(1);
    let mut n = u16::MAX;
    while let Some(field_number) = chunk.get_field_number(n, n_max, is_diff) {
        n = field_number;
        let Some(field) = cdef.fields.get(field_number as usize) else {
            break;
        };
        let name = string_by_index(field.name_index).to_owned();
        let value = load_item(chunk, field.type_index, is_diff, class_defs);
        result.insert(name, value);
    }
    Some(result)
}

fn load_item(
    chunk: &mut Chunk<'_>,
    item_type: u32,
    is_diff: bool,
    class_defs: &ClassDefMap<'_>,
) -> Value {
    match item_type {
        ST_STRING => Value::String(chunk.read_string().unwrap_or_default()),
        ST_BOOL => Value::Bool(chunk.read_bool().unwrap_or(false)),
        ST_INT8 => {
            let value = chunk.read_u8().unwrap_or(0);
            Value::Number(Number::from(if value > 127 {
                i64::from(value) - 256
            } else {
                i64::from(value)
            }))
        }
        ST_UINT8 => Value::Number(Number::from(chunk.read_u8().unwrap_or(0))),
        ST_INT16 => {
            let value = chunk.read_u16().unwrap_or(0);
            Value::Number(Number::from(if value > 32767 {
                i64::from(value) - 65536
            } else {
                i64::from(value)
            }))
        }
        ST_UINT16 => Value::Number(Number::from(chunk.read_u16().unwrap_or(0))),
        ST_INT32 => Value::Number(Number::from(chunk.read_i32().unwrap_or(0))),
        ST_UINT32 => Value::Number(Number::from(chunk.read_u32().unwrap_or(0))),
        ST_INT64 => Value::Number(Number::from(chunk.read_i64().unwrap_or(0))),
        ST_UINT64 => Value::Number(Number::from(chunk.read_u64().unwrap_or(0))),
        ST_FLOAT => number_from_f64(f64::from(chunk.read_float().unwrap_or(0.0))),
        ST_DOUBLE => number_from_f64(chunk.read_double().unwrap_or(0.0)),
        ST_LIST => Value::Array(Vec::new()),
        ST_MAP => Value::Object(Map::new()),
        ST_REF => {
            let _ = chunk.read_u32();
            Value::Null
        }
        ST_BSCOMPONENTDB2_ID => load_component_db_id(chunk, is_diff, class_defs),
        item_type if item_type > ST_UNKNOWN => {
            let class_name = string_by_index(item_type);
            let Some(cdef) = class_defs.get(class_name) else {
                return Value::Null;
            };
            if (cdef.class_flags & 4) != 0 {
                return Value::Null;
            }
            walk_fields(chunk, cdef, is_diff, class_defs)
                .map(Value::Object)
                .unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

fn load_component_db_id(
    chunk: &mut Chunk<'_>,
    is_diff: bool,
    class_defs: &ClassDefMap<'_>,
) -> Value {
    let Some(cdef) = class_defs.get("BSComponentDB2::ID") else {
        return Value::Number(Number::from(chunk.read_u32().unwrap_or(0)));
    };
    if cdef.field_count == 1
        && cdef
            .fields
            .first()
            .is_some_and(|field| field.type_index == ST_UINT32)
        && chunk.get_field_number(u16::MAX, 0, is_diff).is_some()
    {
        return Value::Number(Number::from(chunk.read_u32().unwrap_or(0)));
    }
    Value::Number(Number::from(chunk.read_u32().unwrap_or(0)))
}

fn populate_material_props(props: &mut Ce2MaterialPropsPayload, data: &Map<String, Value>) {
    if let Some(color) = get_color(data, "Color") {
        props.emissive_color = color;
    }
    if let Some(value) = get_f32(data, "fGlossiness") {
        props.smoothness = value;
    }
    if let Some(value) = get_f32(data, "fMetalness") {
        props.metalness = value;
    }
    if let Some(value) = get_f32(data, "fAlpha") {
        props.alpha = value;
    }
}

fn current_layer_mut<'a>(
    material: &'a mut Ce2MaterialPayload,
    state: &WalkState,
) -> Option<&'a mut Ce2LayerPayload> {
    material.layers.get_mut(state.current_layer?)
}

fn get_string(data: &Map<String, Value>, name: &str) -> Option<String> {
    data.get(name)?.as_str().map(str::to_owned)
}

fn get_f32(data: &Map<String, Value>, name: &str) -> Option<f32> {
    data.get(name)?.as_f64().map(|value| value as f32)
}

fn get_i32(data: &Map<String, Value>, name: &str) -> Option<i32> {
    data.get(name)
        .and_then(|value| value.as_i64().or_else(|| value.as_u64().map(|v| v as i64)))
        .map(|value| value as i32)
}

fn get_color(data: &Map<String, Value>, name: &str) -> Option<[f32; 3]> {
    let color = data.get(name)?.as_object()?;
    Some([
        color.get("Red")?.as_f64()? as f32,
        color.get("Green")?.as_f64()? as f32,
        color.get("Blue")?.as_f64()? as f32,
    ])
}

fn component_index(component: &ComponentBlobPayload) -> u32 {
    component.key & 0xFFFF
}

fn class_def_map(class_defs: &[ClassDefPayload]) -> ClassDefMap<'_> {
    class_defs
        .iter()
        .map(|class_def| (class_def.class_name.as_str(), class_def))
        .collect()
}

fn string_by_index(index: u32) -> &'static str {
    STRING_TABLE
        .get(index as usize)
        .copied()
        .unwrap_or(STRING_TABLE[18])
}

fn number_from_f64(value: f64) -> Value {
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn default_material_payload() -> Ce2MaterialPayload {
    Ce2MaterialPayload {
        name: String::new(),
        object_path: String::new(),
        layers: Vec::new(),
        blenders: Vec::new(),
        lod_materials: Vec::new(),
    }
}

fn default_layer_payload() -> Ce2LayerPayload {
    Ce2LayerPayload {
        material: Ce2MaterialPropsPayload {
            smoothness: 0.5,
            metalness: 0.0,
            emissive_color: [0.0, 0.0, 0.0],
            emissive_multiplier: 0.0,
            alpha: 1.0,
        },
        texture_set: Ce2TextureSetPayload {
            diffuse: String::new(),
            normal: String::new(),
            opacity: String::new(),
            rough: String::new(),
            metal: String::new(),
            ao: String::new(),
            emissive: String::new(),
        },
        uv_stream: Ce2UvStreamPayload {
            scale_u: 1.0,
            scale_v: 1.0,
            offset_u: 0.0,
            offset_v: 0.0,
            channel: 0,
        },
        name: String::new(),
    }
}

fn default_blender_payload() -> Ce2BlenderPayload {
    Ce2BlenderPayload {
        mode: 0,
        mask_texture: String::new(),
    }
}
