use std::collections::{BTreeMap, HashMap, HashSet};

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
    pub conversion_blockers: Vec<String>,
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
    pub replacements: BTreeMap<u32, [f32; 4]>,
    pub normal_intensity: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2UvStreamPayload {
    pub scale_u: f32,
    pub scale_v: f32,
    pub offset_u: f32,
    pub offset_v: f32,
    pub channel: i32,
    pub address_mode: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2MaterialPropsPayload {
    pub smoothness: f32,
    pub metalness: f32,
    pub emissive_color: [f32; 3],
    pub emissive_multiplier: f32,
    pub alpha: f32,
    pub color: [f32; 4],
    pub color_mode: i32,
    pub use_vertex_color: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ce2BlenderPayload {
    pub mode: i32,
    pub mask_texture: String,
    pub mask_replacement: Option<[f32; 4]>,
    pub uv_stream: Ce2UvStreamPayload,
    pub color_channel: i32,
    pub float_params: [f32; 5],
    pub bool_params: [bool; 8],
}

type ClassDefMap<'a> = HashMap<&'a str, &'a ClassDefPayload>;

pub fn project(cdb: &CdbPayload, root_db_id: u32) -> Option<Ce2MaterialPayload> {
    let mut graph = MaterialGraph {
        objects: cdb.objects.iter().map(|obj| (obj.db_id, obj)).collect(),
        class_defs: class_def_map(&cdb.class_defs),
        resolved: HashMap::new(),
        resolving: HashSet::new(),
    };
    graph.project(root_db_id, &mut HashSet::new())
}

type Components = BTreeMap<(String, u32), Map<String, Value>>;

struct MaterialGraph<'a> {
    objects: HashMap<u32, &'a MaterialObjectPayload>,
    class_defs: ClassDefMap<'a>,
    resolved: HashMap<u32, Components>,
    resolving: HashSet<u32>,
}

impl MaterialGraph<'_> {
    fn components(&mut self, id: u32) -> Option<Components> {
        if let Some(components) = self.resolved.get(&id) {
            return Some(components.clone());
        }
        if !self.resolving.insert(id) {
            return None;
        }
        let object = *self.objects.get(&id)?;
        let mut components = if object.base_object_db_id == 0 {
            Components::new()
        } else {
            self.components(object.base_object_db_id)?
        };
        for component in &object.components {
            let key = (component.class_name.clone(), component_index(component));
            let data = walk_component_map(component, &self.class_defs)?;
            if component.is_diff {
                merge_fields(components.entry(key).or_default(), data);
            } else {
                components.insert(key, data);
            }
        }
        self.resolving.remove(&id);
        self.resolved.insert(id, components.clone());
        Some(components)
    }

    fn referenced(&mut self, components: &Components, class: &str) -> Option<Components> {
        match reference_id(components, class, 0) {
            Some(id) if id != 0 => self.components(id),
            _ => Some(Components::new()),
        }
    }

    fn project(&mut self, id: u32, visiting: &mut HashSet<u32>) -> Option<Ce2MaterialPayload> {
        if !visiting.insert(id) {
            return None;
        }
        let components = self.components(id)?;
        let mut material = default_material_payload();
        material.name = component_name(&components);
        material.object_path = material.name.clone();
        collect_emissive_blockers(&components, &mut material.conversion_blockers);
        for ((class, _), data) in &components {
            let Some(target) = data.get("ID").and_then(Value::as_u64) else {
                continue;
            };
            if target == 0 {
                continue;
            }
            match class.as_str() {
                "BSMaterial::LayerID" => material
                    .layers
                    .push(self.layer(target as u32, &mut material.conversion_blockers)?),
                "BSMaterial::BlenderID" => {
                    let blender_components = self.components(target as u32)?;
                    let mut blender = default_blender_payload();
                    blender.mask_texture =
                        component_data(&blender_components, "BSMaterial::MRTextureFile", 0)
                            .and_then(|data| get_string(data, "FileName"))
                            .unwrap_or_default();
                    populate_blender(
                        &mut blender,
                        &blender_components,
                        &mut material.conversion_blockers,
                    );
                    blender.uv_stream = project_uv(
                        &self.referenced(&blender_components, "BSMaterial::UVStreamID")?,
                        &mut material.conversion_blockers,
                    );
                    material.blenders.push(blender);
                }
                "BSMaterial::LODMaterialID" => material
                    .lod_materials
                    .push(self.project(target as u32, visiting)?),
                _ => {}
            }
        }
        visiting.remove(&id);
        Some(material)
    }

    fn layer(&mut self, id: u32, blockers: &mut Vec<String>) -> Option<Ce2LayerPayload> {
        let components = self.components(id)?;
        let mut layer = default_layer_payload();
        layer.name = component_name(&components);
        let material = self.referenced(&components, "BSMaterial::MaterialID")?;
        collect_emissive_blockers(&material, blockers);
        for data in material.values() {
            populate_material_props(&mut layer.material, data);
        }
        if let Some(data) = component_data(&material, "BSMaterial::Color", 0) {
            layer.material.color = rgba(data.get("Value")?, [1.0, 1.0, 1.0, 0.0])?;
        }
        layer.material.color_mode = enum_component(
            &material,
            "BSMaterial::MaterialOverrideColorTypeComponent",
            &["Multiply", "Lerp"],
            1,
            blockers,
        );
        layer.material.use_vertex_color = component_data(&material, "BSMaterial::ParamBool", 0)
            .and_then(|data| data.get("Value"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if layer.material.use_vertex_color
            || (layer.material.color_mode == 0 && layer.material.color[..3] != [1.0; 3])
            || (layer.material.color_mode == 1 && layer.material.color[3] != 0.0)
        {
            blockers.push("material color requires baking".into());
        }
        let textures = self.referenced(&material, "BSMaterial::TextureSetID")?;
        if textures.iter().any(|((class, _), data)| {
            class == "BSMaterial::TextureReplacement"
                && data.get("Enabled").and_then(Value::as_bool) == Some(true)
        }) {
            blockers.push("texture replacement requires baking".to_owned());
        }
        for ((class, index), data) in &textures {
            if class == "BSMaterial::TextureReplacement"
                && data.get("Enabled").and_then(Value::as_bool) == Some(true)
            {
                let default = default_texture_replacement(*index)?;
                let color = match data.get("Color").and_then(|color| color.get("Value")) {
                    Some(value) => rgba(value, default)?,
                    None => default,
                };
                layer.texture_set.replacements.insert(*index, color);
            }
            if class != "BSMaterial::MRTextureFile" {
                continue;
            }
            let Some(filename) = get_string(data, "FileName") else {
                continue;
            };
            match index {
                0 => layer.texture_set.diffuse = filename,
                1 => layer.texture_set.normal = filename,
                2 => layer.texture_set.opacity = filename,
                3 => layer.texture_set.rough = filename,
                4 => layer.texture_set.metal = filename,
                5 => layer.texture_set.ao = filename,
                7 => layer.texture_set.emissive = filename,
                _ => {}
            }
        }
        layer.texture_set.normal_intensity =
            component_data(&textures, "BSMaterial::MaterialParamFloat", 0)
                .and_then(|data| get_f32(data, "Value"))
                .unwrap_or(1.0);
        if layer.texture_set.normal_intensity != 1.0 {
            blockers.push("normal intensity requires baking".into());
        }
        layer.uv_stream = project_uv(
            &self.referenced(&components, "BSMaterial::UVStreamID")?,
            blockers,
        );
        Some(layer)
    }
}

fn rgba(value: &Value, mut color: [f32; 4]) -> Option<[f32; 4]> {
    value.as_object()?;
    for (channel, name) in color.iter_mut().zip(["x", "y", "z", "w"]) {
        let Some(value) = value.get(name) else {
            continue;
        };
        *channel = value.as_f64()? as f32;
        if !channel.is_finite() {
            return None;
        }
        *channel = channel.clamp(0.0, 1.0);
    }
    Some(color)
}

fn default_texture_replacement(index: u32) -> Option<[f32; 4]> {
    let packed = *[
        0xff000000_u32,
        0xffff8080,
        0xffffffff,
        0xff000000,
        0xff000000,
        0xffffffff,
        0xff000000,
        0xff000000,
        0xff000000,
        0xff808080,
        0xff000000,
        0xff808080,
        0xff000000,
        0xff000000,
        0xff808080,
        0xff808080,
        0xff808080,
        0xff000000,
        0xff000000,
        0xffffffff,
        0xff808080,
    ]
    .get(index as usize)?;
    Some(packed.to_le_bytes().map(|channel| channel as f32 / 255.0))
}

fn enum_component(
    components: &Components,
    class: &str,
    names: &[&str],
    default: i32,
    blockers: &mut Vec<String>,
) -> i32 {
    let Some(data) = component_data(components, class, 0) else {
        return default;
    };
    match data
        .get("Value")
        .and_then(Value::as_str)
        .and_then(|value| names.iter().position(|name| *name == value))
    {
        Some(index) => index as i32,
        None => {
            blockers.push(format!("unsupported {class} value"));
            default
        }
    }
}

fn project_uv(components: &Components, blockers: &mut Vec<String>) -> Ce2UvStreamPayload {
    let mut uv = default_uv_payload();
    for (class, x, y) in [
        ("BSMaterial::Scale", &mut uv.scale_u, &mut uv.scale_v),
        ("BSMaterial::Offset", &mut uv.offset_u, &mut uv.offset_v),
    ] {
        if let Some(value) = component_data(components, class, 0).and_then(|data| data.get("Value"))
        {
            match (
                value.get("x").and_then(Value::as_f64),
                value.get("y").and_then(Value::as_f64),
            ) {
                (Some(a), Some(b)) if (a as f32).is_finite() && (b as f32).is_finite() => {
                    *x = a as f32;
                    *y = b as f32;
                }
                _ => blockers.push(format!("invalid {class} value")),
            }
        }
    }
    uv.channel = enum_component(
        components,
        "BSMaterial::Channel",
        &["Zero", "One", "Two", "Three"],
        0,
        blockers,
    );
    uv.address_mode = enum_component(
        components,
        "BSMaterial::TextureAddressModeComponent",
        &["Wrap", "Clamp", "Mirror", "Border"],
        0,
        blockers,
    );
    if uv.channel != 0 {
        blockers.push("nonzero UV channel requires mesh projection".into());
    }
    if uv.address_mode > 1 {
        blockers.push("mirror/border UV addressing requires baking".into());
    }
    uv
}

fn populate_blender(
    blender: &mut Ce2BlenderPayload,
    components: &Components,
    blockers: &mut Vec<String>,
) {
    blender.mode = enum_component(
        components,
        "BSMaterial::BlendModeComponent",
        &[
            "Linear",
            "Additive",
            "PositionContrast",
            "None",
            "CharacterCombine",
            "Skin",
        ],
        0,
        blockers,
    );
    blender.color_channel = enum_component(
        components,
        "BSMaterial::ColorChannelTypeComponent",
        &["Red", "Green", "Blue", "Alpha"],
        0,
        blockers,
    );
    for ((class, index), data) in components {
        if class == "BSMaterial::MaterialParamFloat"
            && (*index as usize) < blender.float_params.len()
        {
            match get_f32(data, "Value") {
                Some(value) if value.is_finite() => blender.float_params[*index as usize] = value,
                _ => blockers.push("invalid blender float parameter".into()),
            }
        } else if class == "BSMaterial::ParamBool" && (*index as usize) < blender.bool_params.len()
        {
            match data.get("Value").and_then(Value::as_bool) {
                Some(value) => blender.bool_params[*index as usize] = value,
                None => blockers.push("invalid blender boolean parameter".into()),
            }
        } else if class == "BSMaterial::TextureReplacement"
            && data.get("Enabled").and_then(Value::as_bool) == Some(true)
        {
            blender.mask_replacement = match data.get("Color").and_then(|value| value.get("Value"))
            {
                Some(value) => rgba(value, [1.0; 4]),
                None => Some([1.0; 4]),
            };
            if blender.mask_replacement.is_none() {
                blockers.push("invalid blender mask replacement".into());
            }
        }
    }
}

fn component_data<'a>(
    components: &'a Components,
    class: &str,
    index: u32,
) -> Option<&'a Map<String, Value>> {
    components.get(&(class.to_owned(), index))
}

fn collect_emissive_blockers(components: &Components, blockers: &mut Vec<String>) {
    for ((class, _), data) in components {
        if matches!(
            class.as_str(),
            "BSMaterial::EmissiveSettingsComponent" | "BSMaterial::LayeredEmissivityComponent"
        ) && data.get("Enabled").and_then(Value::as_bool) != Some(false)
        {
            blockers.push(format!("{class} requires verified emission projection"));
        }
    }
}

fn reference_id(components: &Components, class: &str, index: u32) -> Option<u32> {
    component_data(components, class, index)?
        .get("ID")?
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
}

fn component_name(components: &Components) -> String {
    component_data(components, "BSComponentDB::CTName", 0)
        .and_then(|data| get_string(data, "Name"))
        .unwrap_or_default()
}

fn merge_fields(target: &mut Map<String, Value>, source: Map<String, Value>) {
    for (key, value) in source {
        if let (Some(Value::Object(target)), Value::Object(source)) = (target.get_mut(&key), &value)
        {
            merge_fields(target, source.clone());
        } else {
            target.insert(key, value);
        }
    }
}

pub fn walk_component(
    component: &ComponentBlobPayload,
    class_defs: &[ClassDefPayload],
) -> Option<Value> {
    let class_defs = class_def_map(class_defs);
    walk_component_map(component, &class_defs).map(Value::Object)
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

fn get_string(data: &Map<String, Value>, name: &str) -> Option<String> {
    data.get(name)?.as_str().map(str::to_owned)
}

fn get_f32(data: &Map<String, Value>, name: &str) -> Option<f32> {
    data.get(name)?.as_f64().map(|value| value as f32)
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
        conversion_blockers: Vec::new(),
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
            color: [1.0, 1.0, 1.0, 0.0],
            color_mode: 1,
            use_vertex_color: false,
        },
        texture_set: Ce2TextureSetPayload {
            diffuse: String::new(),
            normal: String::new(),
            opacity: String::new(),
            rough: String::new(),
            metal: String::new(),
            ao: String::new(),
            emissive: String::new(),
            replacements: BTreeMap::new(),
            normal_intensity: 1.0,
        },
        uv_stream: default_uv_payload(),
        name: String::new(),
    }
}

fn default_uv_payload() -> Ce2UvStreamPayload {
    Ce2UvStreamPayload {
        scale_u: 1.0,
        scale_v: 1.0,
        offset_u: 0.0,
        offset_v: 0.0,
        channel: 0,
        address_mode: 0,
    }
}

fn default_blender_payload() -> Ce2BlenderPayload {
    Ce2BlenderPayload {
        mode: 0,
        mask_texture: String::new(),
        mask_replacement: None,
        uv_stream: default_uv_payload(),
        color_channel: 0,
        float_params: [0.5, 0.5, 0.5, 1.0, 1.0],
        bool_params: [true, true, true, true, true, false, true, false],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdb::{FieldDefPayload, ResourceId};

    fn class(name: &str, field: &str, field_type: u32) -> ClassDefPayload {
        ClassDefPayload {
            class_name: name.to_owned(),
            class_name_index: 0,
            class_version: 1,
            class_flags: 0,
            field_count: 1,
            fields: vec![FieldDefPayload {
                name_index: STRING_TABLE.iter().position(|name| *name == field).unwrap() as u32,
                type_index: field_type,
                data_offset: 0,
                data_size: 0,
            }],
        }
    }

    fn reference(class: &str, id: u32) -> ComponentBlobPayload {
        ComponentBlobPayload {
            class_name: class.to_owned(),
            is_diff: false,
            key: 0,
            body: id.to_le_bytes().to_vec(),
        }
    }

    fn texture(index: u32, path: &str) -> ComponentBlobPayload {
        let mut body = (path.len() as u16).to_le_bytes().to_vec();
        body.extend(path.as_bytes());
        ComponentBlobPayload {
            class_name: "BSMaterial::MRTextureFile".to_owned(),
            is_diff: false,
            key: index,
            body,
        }
    }

    fn object(id: u32, base: u32, components: Vec<ComponentBlobPayload>) -> MaterialObjectPayload {
        MaterialObjectPayload {
            persistent_id: ResourceId {
                dir: 0,
                file: 0,
                ext: 0,
            },
            db_id: id,
            base_object_db_id: base,
            has_data: true,
            parent_db_id: Some(999),
            components,
        }
    }

    fn graph() -> CdbPayload {
        CdbPayload {
            class_defs: vec![
                class("BSMaterial::LayerID", "ID", ST_UINT32),
                class("BSMaterial::MaterialID", "ID", ST_UINT32),
                class("BSMaterial::TextureSetID", "ID", ST_UINT32),
                class("BSMaterial::MRTextureFile", "FileName", ST_STRING),
            ],
            objects: vec![
                object(5, 4, vec![texture(1, "child_normal.dds")]),
                object(1, 0, vec![reference("BSMaterial::LayerID", 2)]),
                object(3, 0, vec![reference("BSMaterial::TextureSetID", 5)]),
                object(
                    4,
                    0,
                    vec![
                        texture(3, "rough.dds"),
                        texture(0, "base_color.dds"),
                        texture(1, "base_normal.dds"),
                    ],
                ),
                object(2, 0, vec![reference("BSMaterial::MaterialID", 3)]),
                object(6, 0, vec![texture(0, "unreferenced.dds")]),
            ],
            component_info: vec![],
        }
    }

    #[test]
    fn projects_authored_blender_parameters_without_replacing_them_with_defaults() {
        let components: Components = [
            (("BSMaterial::BlendModeComponent".into(), 0), serde_json::json!({"Value":"CharacterCombine"})),
            (("BSMaterial::ColorChannelTypeComponent".into(), 0), serde_json::json!({"Value":"Blue"})),
            (("BSMaterial::MaterialParamFloat".into(), 4), serde_json::json!({"Value":0.25})),
            (("BSMaterial::ParamBool".into(), 3), serde_json::json!({"Value":false})),
            (("BSMaterial::TextureReplacement".into(), 0), serde_json::json!({"Enabled":true,"Color":{"Value":{"x":0.6,"y":0.4,"z":0.2,"w":1.0}}})),
        ].into_iter().map(|(key,value)| (key, value.as_object().unwrap().clone())).collect();
        let mut blender = default_blender_payload();
        let mut blockers = Vec::new();
        populate_blender(&mut blender, &components, &mut blockers);
        assert_eq!(blender.mode, 4);
        assert_eq!(blender.color_channel, 2);
        assert_eq!(blender.float_params, [0.5, 0.5, 0.5, 1.0, 0.25]);
        assert!(!blender.bool_params[3]);
        assert_eq!(blender.mask_replacement, Some([0.6, 0.4, 0.2, 1.0]));
        assert!(blockers.is_empty());
    }

    #[test]
    fn projects_uv_offsets_clamping_and_rejects_unknown_channels() {
        let mut components: Components = [
            (
                ("BSMaterial::Scale".into(), 0),
                serde_json::json!({"Value":{"x":40.0,"y":20.0}}),
            ),
            (
                ("BSMaterial::Offset".into(), 0),
                serde_json::json!({"Value":{"x":0.25,"y":-0.5}}),
            ),
            (
                ("BSMaterial::Channel".into(), 0),
                serde_json::json!({"Value":"Zero"}),
            ),
            (
                ("BSMaterial::TextureAddressModeComponent".into(), 0),
                serde_json::json!({"Value":"Clamp"}),
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key, value.as_object().unwrap().clone()))
        .collect();
        let mut blockers = Vec::new();
        let uv = project_uv(&components, &mut blockers);
        assert_eq!(
            [uv.scale_u, uv.scale_v, uv.offset_u, uv.offset_v],
            [40.0, 20.0, 0.25, -0.5]
        );
        assert_eq!(uv.address_mode, 1);
        assert!(blockers.is_empty());
        components
            .get_mut(&("BSMaterial::Channel".into(), 0))
            .unwrap()
            .insert("Value".into(), Value::String("Four".into()));
        project_uv(&components, &mut blockers);
        assert!(
            blockers
                .iter()
                .any(|value| value.contains("unsupported BSMaterial::Channel"))
        );
    }

    #[test]
    fn resolves_references_inheritance_and_sparse_texture_indices() {
        let material = project(&graph(), 1).unwrap();
        assert_eq!(material.layers.len(), 1);
        let textures = &material.layers[0].texture_set;
        assert_eq!(textures.diffuse, "base_color.dds");
        assert_eq!(textures.normal, "child_normal.dds");
        assert_eq!(textures.rough, "rough.dds");
        assert!(textures.metal.is_empty());
    }

    #[test]
    fn rejects_inheritance_cycles_and_dangling_references() {
        let mut source = graph();
        source
            .objects
            .iter_mut()
            .find(|object| object.db_id == 4)
            .unwrap()
            .base_object_db_id = 5;
        assert!(project(&source, 1).is_none());
        source.objects.retain(|object| object.db_id != 2);
        assert!(project(&source, 1).is_none());
    }

    #[test]
    fn component_diffs_preserve_unmodified_nested_fields() {
        let mut base = serde_json::json!({"Color":{"Value":{"x":0.5,"y":0.7}},"Enabled":true})
            .as_object()
            .unwrap()
            .clone();
        merge_fields(
            &mut base,
            serde_json::json!({"Color":{"Value":{"x":0.8}}})
                .as_object()
                .unwrap()
                .clone(),
        );
        assert_eq!(base["Color"]["Value"]["x"], 0.8);
        assert_eq!(base["Color"]["Value"]["y"], 0.7);
        assert_eq!(base["Enabled"], true);
    }
}
