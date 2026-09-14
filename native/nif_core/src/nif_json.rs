use std::path::Path;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::model::{NifBlock, NifFile, NifValue};
use crate::schema::{FieldDef, SCHEMA};

pub fn convert_json_file(input: &Path, output: &Path, options: &Value) -> Result<Value, String> {
    let direction = options
        .get("json_direction")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if input
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            {
                "from-json"
            } else {
                "to-json"
            }
        });
    let digits = options
        .get("decimal_digits")
        .and_then(Value::as_u64)
        .unwrap_or(8) as usize;
    let rotation_euler = options
        .get("rotation_output")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("euler"));
    if !(6..=16).contains(&digits) {
        return Err("Decimal digits can vary from 6 to 16".to_string());
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let game = match direction {
        "to-json" => {
            let nif = NifFile::load(input).map_err(|error| error.to_string())?;
            let value = nif_to_json(&nif, digits, rotation_euler);
            let bytes = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
            std::fs::write(output, bytes).map_err(|error| error.to_string())?;
            crate::validation::nif_game_label(&nif)
        }
        "from-json" => {
            let value: Value =
                serde_json::from_slice(&std::fs::read(input).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?;
            let mut nif = nif_from_json(&value, rotation_euler)?;
            let game = crate::validation::nif_game_label(&nif);
            nif.save(Some(output.to_path_buf()))
                .map_err(|error| error.to_string())?;
            game
        }
        _ => return Err(format!("Unsupported JSON direction: {direction}")),
    };
    Ok(json!({
        "processor": "json-converter",
        "path": input,
        "output": output,
        "game": game,
        "report_only": false,
        "changed": true,
        "changes": [format!("Converted {direction}")],
    }))
}

pub fn nif_to_json(nif: &NifFile, digits: usize, rotation_euler: bool) -> Value {
    let mut root = Map::new();
    root.insert("NiHeader".to_string(), header_json(nif));
    for block in &nif.blocks {
        let mut fields = Map::new();
        for (name, value) in &block.fields {
            let type_name = field_definition(&block.type_name, name).map(|field| field.type_name);
            fields.insert(
                bare_name(name).to_string(),
                nif_value_json(nif, value, type_name, name, digits, rotation_euler),
            );
        }
        root.insert(
            format!("{} {}", block.block_id, block.type_name),
            Value::Object(fields),
        );
    }
    root.insert(
        "NiFooter".to_string(),
        json!({
            "Roots": nif.header.footer_roots.iter().map(|reference| {
                Value::String(reference_text(nif, *reference))
            }).collect::<Vec<_>>()
        }),
    );
    Value::Object(root)
}

pub fn nif_from_json(value: &Value, rotation_euler: bool) -> Result<NifFile, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "NIF JSON root must be an object".to_string())?;
    let header = root
        .get("NiHeader")
        .and_then(Value::as_object)
        .ok_or_else(|| "First block must be NiHeader".to_string())?;
    let version = header
        .get("Version")
        .and_then(json_text)
        .and_then(parse_version)
        .unwrap_or((20, 2, 0, 7));
    let user_version = header.get("User Version").and_then(json_u64).unwrap_or(12) as u32;
    let bs_version = header
        .get("User Version 2")
        .or_else(|| header.get("BS Version"))
        .and_then(json_u64)
        .unwrap_or(0) as u32;
    let game = infer_game(version, user_version, bs_version);
    let mut nif = NifFile::new(game);
    nif.blocks.clear();
    nif.header.block_type_names.clear();
    nif.header.block_type_index.clear();
    nif.header.block_sizes.clear();
    nif.header.footer_roots.clear();
    nif.header.num_blocks = 0;
    nif.header.version = version;
    nif.header.version_packed = pack_version(version);
    nif.header.user_version = user_version;
    nif.header.bs_version = bs_version;
    nif.header.endian_type = header
        .get("Endian Type")
        .and_then(json_text)
        .map(|value| u8::from(!value.eq_ignore_ascii_case("ENDIAN_BIG")))
        .unwrap_or(1);
    nif.header.header_string = header
        .get("Magic")
        .and_then(json_text)
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Gamebryo File Format, Version {}.{}.{}.{}",
                version.0, version.1, version.2, version.3
            )
        });
    if let Some(export) = header.get("Export Info").and_then(Value::as_object) {
        nif.header.creator = export
            .get("Author")
            .and_then(json_text)
            .unwrap_or_default()
            .to_string();
        nif.header.export_info = ["Process Script", "Export Script", "Max Filepath"]
            .into_iter()
            .filter_map(|name| export.get(name).and_then(json_text).map(str::to_string))
            .collect();
    }

    let mut blocks = root
        .iter()
        .filter_map(|(name, fields)| parse_block_key(name).map(|parsed| (parsed, fields)))
        .collect::<Vec<_>>();
    blocks.sort_by_key(|((index, _), _)| *index);
    for (expected, ((index, type_name), fields)) in blocks.into_iter().enumerate() {
        if index != expected {
            return Err(format!(
                "JSON block index {index} is out of order; expected {expected}"
            ));
        }
        let block_id = nif.add_block(type_name, None);
        let object = fields
            .as_object()
            .ok_or_else(|| format!("Block {index} must be an object"))?;
        apply_block_json(&mut nif.blocks[block_id], object, rotation_euler)?;
    }
    if nif.blocks.is_empty() {
        return Err("JSON contains no NIF blocks".to_string());
    }
    nif.header.footer_roots = root
        .get("NiFooter")
        .and_then(|footer| footer.get("Roots"))
        .and_then(Value::as_array)
        .map(|roots| roots.iter().filter_map(parse_reference).collect())
        .unwrap_or_else(|| vec![0]);
    nif.raw_block_context = None;
    Ok(nif)
}

fn header_json(nif: &NifFile) -> Value {
    let export = json!({
        "Author": nif.header.creator,
        "Process Script": nif.header.export_info.first().cloned().unwrap_or_default(),
        "Export Script": nif.header.export_info.get(1).cloned().unwrap_or_default(),
        "Max Filepath": nif.header.export_info.get(2).cloned().unwrap_or_default(),
    });
    json!({
        "Magic": nif.header.header_string.trim_end_matches(['\r', '\n']),
        "Version": format!("{}.{}.{}.{}", nif.header.version.0, nif.header.version.1, nif.header.version.2, nif.header.version.3),
        "Endian Type": if nif.header.endian_type == 0 { "ENDIAN_BIG" } else { "ENDIAN_LITTLE" },
        "User Version": nif.header.user_version.to_string(),
        "Num Blocks": nif.blocks.len().to_string(),
        "User Version 2": nif.header.bs_version.to_string(),
        "Export Info": export,
        "Block Types": nif.header.block_type_names.iter().map(|value| Value::String(value.clone())).collect::<Vec<_>>(),
        "Block Type Index": nif.blocks.iter().map(|block| Value::String(block.type_name.clone())).collect::<Vec<_>>(),
        "Block Size": nif.header.block_sizes.iter().map(|value| Value::String(value.to_string())).collect::<Vec<_>>(),
        "Num Strings": nif.header.strings.len().to_string(),
        "Max String Length": nif.header.max_string_length.to_string(),
        "Strings": nif.header.strings,
        "Num Groups": nif.header.num_groups.to_string(),
    })
}

fn nif_value_json(
    nif: &NifFile,
    value: &NifValue,
    type_name: Option<&str>,
    field_name: &str,
    digits: usize,
    rotation_euler: bool,
) -> Value {
    if let Some(type_name) = type_name {
        if let Some(value) = merged_value_json(value, type_name, field_name, digits, rotation_euler)
        {
            return value;
        }
    }
    match value {
        NifValue::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| {
                    nif_value_json(nif, value, type_name, field_name, digits, rotation_euler)
                })
                .collect(),
        ),
        NifValue::Struct(fields) => {
            let mut object = Map::new();
            for (name, value) in fields {
                let nested_type = type_name
                    .and_then(|type_name| field_definition(type_name, name))
                    .map(|field| field.type_name);
                object.insert(
                    bare_name(name).to_string(),
                    nif_value_json(nif, value, nested_type, name, digits, rotation_euler),
                );
            }
            Value::Object(object)
        }
        NifValue::Vec3(values) => compact_object(&["x", "y", "z"], values, digits),
        NifValue::Vec4(values) => compact_object(&["x", "y", "z", "w"], values, digits),
        NifValue::Color3(values) => compact_object(&["r", "g", "b"], values, digits),
        NifValue::Color4(values) => compact_object(&["r", "g", "b", "a"], values, digits),
        NifValue::Quaternion(values) => compact_object(&["w", "x", "y", "z"], values, digits),
        NifValue::Matrix33(matrix) => {
            let values = [
                matrix[0][0],
                matrix[0][1],
                matrix[0][2],
                matrix[1][0],
                matrix[1][1],
                matrix[1][2],
                matrix[2][0],
                matrix[2][1],
                matrix[2][2],
            ];
            compact_object(
                &[
                    "m11", "m21", "m31", "m12", "m22", "m32", "m13", "m23", "m33",
                ],
                &values,
                digits,
            )
        }
        NifValue::Matrix44(matrix) => {
            let values = matrix.iter().flatten().copied().collect::<Vec<_>>();
            compact_object(
                &[
                    "m11", "m12", "m13", "m14", "m21", "m22", "m23", "m24", "m31", "m32", "m33",
                    "m34", "m41", "m42", "m43", "m44",
                ],
                &values,
                digits,
            )
        }
        NifValue::Bytes(bytes) => Value::String(
            bytes
                .iter()
                .map(|value| format!("{value:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        NifValue::Ref(reference) => Value::String(reference_text(nif, *reference)),
        NifValue::Float(value) => Value::String(format_edit_float(*value, digits)),
        NifValue::FloatNan(_) => Value::String("NaN".to_string()),
        NifValue::UInt(value) => Value::String(symbolic_integer(*value as i64, type_name)),
        NifValue::Int(value) => Value::String(symbolic_integer(*value, type_name)),
        NifValue::Bool(value) => Value::String(u8::from(*value).to_string()),
        NifValue::String(value) | NifValue::Char(value) => Value::String(value.clone()),
        NifValue::Null => Value::String(String::new()),
    }
}

fn merged_value_json(
    value: &NifValue,
    type_name: &str,
    field_name: &str,
    digits: usize,
    rotation_euler: bool,
) -> Option<Value> {
    let text = match (type_name, value) {
        ("Vector3" | "HalfVector3" | "ByteVector3", NifValue::Vec3(values)) => {
            joined_floats(values, digits)
        }
        ("Vector4", NifValue::Vec4(values)) => joined_floats(values, digits),
        ("Color3", NifValue::Color3(values)) => color_hex(values),
        ("Color4" | "ByteColor4" | "ByteColor4BGRA", NifValue::Color4(values)) => color_hex(values),
        ("Quaternion", NifValue::Quaternion(values)) => {
            quaternion_text(*values, digits, rotation_euler)
        }
        ("hkQuaternion", NifValue::Struct(fields)) => {
            let values = [
                struct_float(fields, "w")?,
                struct_float(fields, "x")?,
                struct_float(fields, "y")?,
                struct_float(fields, "z")?,
            ];
            quaternion_text(values, digits, rotation_euler)
        }
        ("Matrix33", NifValue::Matrix33(matrix))
            if bare_name(field_name).eq_ignore_ascii_case("Rotation") =>
        {
            matrix_rotation_text(*matrix, digits, rotation_euler)
        }
        ("TexCoord" | "HalfTexCoord" | "Triangle", NifValue::Struct(fields)) => fields
            .values()
            .map(|value| scalar_edit_text(value, digits))
            .collect::<Option<Vec<_>>>()?
            .join(" "),
        _ => return None,
    };
    Some(Value::String(text))
}

fn joined_floats<const N: usize>(values: &[f32; N], digits: usize) -> String {
    values
        .iter()
        .map(|value| format_edit_float(f64::from(*value), digits))
        .collect::<Vec<_>>()
        .join(" ")
}

fn color_hex<const N: usize>(values: &[f32; N]) -> String {
    let mut result = String::from("#");
    for value in values {
        let byte = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        result.push_str(&format!("{byte:02X}"));
    }
    result
}

fn quaternion_text(values: [f32; 4], digits: usize, rotation_euler: bool) -> String {
    if values.iter().all(|value| *value == f32::MIN) {
        return "Min".to_string();
    }
    let quaternion = normalized_quaternion(values);
    if rotation_euler {
        return joined_f64(&matrix_to_euler(quaternion_to_matrix(quaternion)), digits);
    }
    let [w, x, y, z] = quaternion;
    let angle = (2.0 * f64::from(w).clamp(-1.0, 1.0).acos()).to_degrees();
    let scale = (1.0 - f64::from(w) * f64::from(w)).max(0.0).sqrt();
    let axis = if scale <= f64::EPSILON {
        [1.0, 0.0, 0.0]
    } else {
        [
            f64::from(x) / scale,
            f64::from(y) / scale,
            f64::from(z) / scale,
        ]
    };
    joined_f64(&[angle, axis[0], axis[1], axis[2]], digits)
}

fn matrix_rotation_text(matrix: [[f32; 3]; 3], digits: usize, rotation_euler: bool) -> String {
    if rotation_euler {
        joined_f64(&matrix_to_euler(matrix), digits)
    } else {
        quaternion_text(matrix_to_quaternion(matrix), digits, false)
    }
}

fn joined_f64(values: &[f64], digits: usize) -> String {
    values
        .iter()
        .map(|value| format_edit_float(*value, digits))
        .collect::<Vec<_>>()
        .join(" ")
}

fn scalar_edit_text(value: &NifValue, digits: usize) -> Option<String> {
    match value {
        NifValue::Float(value) => Some(format_edit_float(*value, digits)),
        NifValue::UInt(value) => Some(value.to_string()),
        NifValue::Int(value) => Some(value.to_string()),
        NifValue::Bool(value) => Some(u8::from(*value).to_string()),
        NifValue::String(value) | NifValue::Char(value) => Some(value.clone()),
        _ => None,
    }
}

fn struct_float(fields: &IndexMap<String, NifValue>, name: &str) -> Option<f32> {
    fields.iter().find_map(|(field_name, value)| {
        field_name.eq_ignore_ascii_case(name).then(|| match value {
            NifValue::Float(value) => Some(*value as f32),
            NifValue::UInt(value) => Some(*value as f32),
            NifValue::Int(value) => Some(*value as f32),
            _ => None,
        })?
    })
}

fn parse_merged_value(
    text: &str,
    type_name: &str,
    rotation_euler: bool,
) -> Result<Option<NifValue>, String> {
    let parsed = match type_name {
        "Vector3" | "HalfVector3" | "ByteVector3" => Some(NifValue::Vec3(parse_float_array(text)?)),
        "Vector4" => Some(NifValue::Vec4(parse_float_array(text)?)),
        "Color3" => Some(NifValue::Color3(parse_color(text)?)),
        "Color4" | "ByteColor4" | "ByteColor4BGRA" => Some(NifValue::Color4(parse_color(text)?)),
        "Quaternion" => Some(NifValue::Quaternion(parse_quaternion(
            text,
            rotation_euler,
        )?)),
        "hkQuaternion" => {
            let [w, x, y, z] = parse_quaternion(text, rotation_euler)?;
            Some(NifValue::Struct(IndexMap::from([
                ("x".to_string(), NifValue::Float(f64::from(x))),
                ("y".to_string(), NifValue::Float(f64::from(y))),
                ("z".to_string(), NifValue::Float(f64::from(z))),
                ("w".to_string(), NifValue::Float(f64::from(w))),
            ])))
        }
        "Matrix33" => Some(NifValue::Matrix33(parse_rotation_matrix(
            text,
            rotation_euler,
        )?)),
        "TexCoord" | "HalfTexCoord" | "Triangle" => {
            let values = split_values(text);
            let definitions = SCHEMA.get_all_fields(type_name);
            if values.len() < definitions.len() {
                return Err(format!(
                    "{type_name} needs {} values, found {}",
                    definitions.len(),
                    values.len()
                ));
            }
            let mut fields = IndexMap::new();
            for (definition, value) in definitions.iter().zip(values) {
                fields.insert(
                    definition.name.to_string(),
                    parse_typed_value(
                        &Value::String(value.to_string()),
                        definition.type_name,
                        None,
                        rotation_euler,
                    )?,
                );
            }
            Some(NifValue::Struct(fields))
        }
        _ => None,
    };
    Ok(parsed)
}

fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, String> {
    let digits = text
        .chars()
        .filter(|character| !matches!(character, ' ' | ',' | ';'))
        .collect::<String>();
    if digits.len() % 2 != 0
        || !digits
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("Invalid hexadecimal byte string: {text}"));
    }
    (0..digits.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&digits[index..index + 2], 16)
                .map_err(|_| format!("Invalid hexadecimal byte string: {text}"))
        })
        .collect()
}

fn parse_float_array<const N: usize>(text: &str) -> Result<[f32; N], String> {
    let values = split_values(text);
    if values.len() < N {
        return Err(format!("Expected {N} values, found {}", values.len()));
    }
    let mut result = [0.0; N];
    for (target, value) in result.iter_mut().zip(values) {
        *target = parse_edit_float(value)? as f32;
    }
    Ok(result)
}

fn parse_color<const N: usize>(text: &str) -> Result<[f32; N], String> {
    if !text.starts_with('#') {
        return parse_float_array(text);
    }
    let mut digits = text[1..].to_string();
    if digits.len() == 3 {
        digits = digits
            .chars()
            .flat_map(|character| [character, character])
            .collect();
    }
    if digits.len() < 6
        || !digits
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("Invalid {N}-channel HTML color: {text}"));
    }
    let mut result = [0.0; N];
    for (index, target) in result.iter_mut().enumerate() {
        if index * 2 + 2 > digits.len() {
            break;
        }
        *target = f32::from(
            u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16)
                .map_err(|_| format!("Invalid HTML color: {text}"))?,
        ) / 255.0;
    }
    Ok(result)
}

fn parse_quaternion(text: &str, rotation_euler: bool) -> Result<[f32; 4], String> {
    if text.eq_ignore_ascii_case("Min") {
        return Ok([f32::MIN; 4]);
    }
    let values = split_values(text);
    if rotation_euler {
        if values.len() < 3 {
            return Err(format!(
                "Euler rotation needs 3 values, found {}",
                values.len()
            ));
        }
        let euler = [
            parse_edit_float(values[0])?,
            parse_edit_float(values[1])?,
            parse_edit_float(values[2])?,
        ];
        return Ok(matrix_to_quaternion(euler_to_matrix(euler)));
    }
    if values.len() < 4 {
        return Err(format!(
            "Angle-axis rotation needs 4 values, found {}",
            values.len()
        ));
    }
    let angle = parse_edit_float(values[0])?.to_radians();
    let mut axis = [
        parse_edit_float(values[1])?,
        parse_edit_float(values[2])?,
        parse_edit_float(values[3])?,
    ];
    let length = axis.iter().map(|value| value * value).sum::<f64>().sqrt();
    if length <= f64::EPSILON {
        axis = [1.0, 0.0, 0.0];
    } else {
        axis.iter_mut().for_each(|value| *value /= length);
    }
    let half = angle * 0.5;
    let sin = half.sin();
    Ok([
        half.cos() as f32,
        (axis[0] * sin) as f32,
        (axis[1] * sin) as f32,
        (axis[2] * sin) as f32,
    ])
}

fn parse_rotation_matrix(text: &str, rotation_euler: bool) -> Result<[[f32; 3]; 3], String> {
    if rotation_euler {
        let values = split_values(text);
        if values.len() < 3 {
            return Err(format!(
                "Euler rotation needs 3 values, found {}",
                values.len()
            ));
        }
        return Ok(euler_to_matrix([
            parse_edit_float(values[0])?,
            parse_edit_float(values[1])?,
            parse_edit_float(values[2])?,
        ]));
    }
    Ok(quaternion_to_matrix(parse_quaternion(text, false)?))
}

fn split_values(value: &str) -> Vec<&str> {
    value.split_whitespace().collect()
}

fn parse_edit_float(value: &str) -> Result<f64, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "nan" => Ok(f64::NAN),
        "inf" => Ok(f64::INFINITY),
        "max" => Ok(f64::from(f32::MAX)),
        "min" => Ok(f64::from(f32::MIN)),
        "" => Ok(0.0),
        _ => value
            .parse()
            .map_err(|_| format!("Invalid floating point value: {value}")),
    }
}

fn normalized_quaternion(quaternion: [f32; 4]) -> [f32; 4] {
    let length = quaternion
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if length <= f32::EPSILON {
        [1.0, 0.0, 0.0, 0.0]
    } else {
        quaternion.map(|value| value / length)
    }
}

fn quaternion_to_matrix(quaternion: [f32; 4]) -> [[f32; 3]; 3] {
    let [w, x, y, z] = normalized_quaternion(quaternion);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

fn matrix_to_quaternion(matrix: [[f32; 3]; 3]) -> [f32; 4] {
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
    normalized_quaternion([w, x, y, z])
}

fn matrix_to_euler(matrix: [[f32; 3]; 3]) -> [f64; 3] {
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

fn euler_to_matrix(euler: [f64; 3]) -> [[f32; 3]; 3] {
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

fn compact_object(names: &[&str], values: &[f32], digits: usize) -> Value {
    Value::Object(
        names
            .iter()
            .zip(values)
            .map(|(name, value)| {
                (
                    (*name).to_string(),
                    Value::String(format_float(*value as f64, digits)),
                )
            })
            .collect(),
    )
}

fn symbolic_integer(value: i64, type_name: Option<&str>) -> String {
    if let Some(definition) = type_name.and_then(|type_name| SCHEMA.get_enum(type_name)) {
        if let Some(option) = definition
            .options
            .iter()
            .find(|option| option.value == value)
        {
            return option.name.to_string();
        }
    }
    if let Some(definition) = type_name.and_then(|type_name| SCHEMA.get_bitflag(type_name)) {
        let names = definition
            .options
            .iter()
            .filter(|option| option.value != 0 && value & option.value == option.value)
            .map(|option| option.name)
            .collect::<Vec<_>>();
        if !names.is_empty() {
            return names.join(" | ");
        }
    }
    value.to_string()
}

fn reference_text(nif: &NifFile, reference: i32) -> String {
    if reference < 0 {
        return "None".to_string();
    }
    let Some(block) = usize::try_from(reference)
        .ok()
        .and_then(|block_id| nif.blocks.get(block_id))
    else {
        return reference.to_string();
    };
    let name = block
        .get_field("Name")
        .and_then(|value| match value {
            NifValue::String(value) | NifValue::Char(value) => Some(value.as_str()),
            _ => None,
        })
        .unwrap_or_default();
    if name.is_empty() {
        format!("{reference} {}", block.type_name)
    } else {
        format!("{reference} {} \"{name}\"", block.type_name)
    }
}

fn apply_block_json(
    block: &mut NifBlock,
    object: &Map<String, Value>,
    rotation_euler: bool,
) -> Result<(), String> {
    for (json_name, json_value) in object {
        let Some(key) = block
            .fields
            .keys()
            .find(|key| bare_name(key).eq_ignore_ascii_case(json_name))
            .cloned()
        else {
            continue;
        };
        let Some(field) = field_definition(&block.type_name, &key) else {
            continue;
        };
        let current = block.fields.get(&key);
        let parsed = parse_json_value(json_value, field, current, rotation_euler)?;
        block.fields.insert(key, parsed);
    }
    Ok(())
}

fn parse_json_value(
    value: &Value,
    field: &FieldDef,
    current: Option<&NifValue>,
    rotation_euler: bool,
) -> Result<NifValue, String> {
    if field.is_binary {
        if let Some(values) = value.as_array() {
            return Ok(NifValue::Bytes(
                values
                    .iter()
                    .map(|value| {
                        json_u64(value)
                            .and_then(|value| u8::try_from(value).ok())
                            .ok_or_else(|| format!("Invalid byte in {}", field.name))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ));
        }
        if let Some(text) = json_text(value) {
            return parse_hex_bytes(text).map(NifValue::Bytes);
        }
    }
    if field.length.is_some() {
        let values = value
            .as_array()
            .ok_or_else(|| format!("{} must be an array", field.name))?;
        return Ok(NifValue::Array(
            values
                .iter()
                .map(|value| parse_typed_value(value, field.type_name, None, rotation_euler))
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }
    parse_typed_value(value, field.type_name, current, rotation_euler)
}

fn parse_typed_value(
    value: &Value,
    type_name: &str,
    current: Option<&NifValue>,
    rotation_euler: bool,
) -> Result<NifValue, String> {
    if matches!(
        type_name,
        "string"
            | "SizedString"
            | "SizedString16"
            | "HeaderString"
            | "LineString"
            | "NiFixedString"
            | "StringPalette"
    ) {
        return Ok(NifValue::String(
            json_text(value).unwrap_or_default().to_string(),
        ));
    }
    if matches!(type_name, "Ref" | "Ptr") {
        return parse_reference(value)
            .map(NifValue::Ref)
            .ok_or_else(|| format!("Invalid reference: {value}"));
    }
    if let Some(definition) = SCHEMA.get_enum(type_name) {
        let text = json_text(value).unwrap_or_default();
        let parsed = definition
            .options
            .iter()
            .find(|option| option.name.eq_ignore_ascii_case(text))
            .map(|option| option.value)
            .or_else(|| parse_i64(text));
        return parsed
            .map(|value| NifValue::UInt(value as u64))
            .ok_or_else(|| format!("Unknown {type_name} value: {text}"));
    }
    if let Some(definition) = SCHEMA.get_bitflag(type_name) {
        let text = json_text(value).unwrap_or_default();
        let mut bits = 0i64;
        for part in text
            .split('|')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            bits |= definition
                .options
                .iter()
                .find(|option| option.name.eq_ignore_ascii_case(part))
                .map(|option| option.value)
                .or_else(|| parse_i64(part))
                .ok_or_else(|| format!("Unknown {type_name} flag: {part}"))?;
        }
        return Ok(NifValue::UInt(bits as u64));
    }
    if SCHEMA.get_struct(type_name).is_some() {
        if let Some(text) = json_text(value) {
            if let Some(parsed) = parse_merged_value(text, type_name, rotation_euler)? {
                return Ok(parsed);
            }
        }
        return parse_struct_value(value, type_name, rotation_euler);
    }
    let text = json_text(value).unwrap_or_default();
    Ok(match type_name {
        "bool" => NifValue::Bool(matches!(
            text.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )),
        "byte" | "ubyte" | "ushort" | "uint" | "ulittle32" | "uint64" | "FileVersion"
        | "StringOffset" | "StringIndex" => NifValue::UInt(
            parse_i64(text)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| format!("Invalid unsigned value: {text}"))?,
        ),
        "sbyte" | "short" | "int" => {
            NifValue::Int(parse_i64(text).ok_or_else(|| format!("Invalid integer value: {text}"))?)
        }
        "float" | "hfloat" => NifValue::Float(parse_edit_float(text)?),
        "char" => NifValue::Char(text.to_string()),
        "string" | "SizedString" | "SizedString16" | "HeaderString" | "LineString"
        | "NiFixedString" | "StringPalette" => NifValue::String(text.to_string()),
        _ => current
            .cloned()
            .unwrap_or_else(|| NifValue::String(text.to_string())),
    })
}

fn parse_struct_value(
    value: &Value,
    type_name: &str,
    rotation_euler: bool,
) -> Result<NifValue, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{type_name} must be an object"))?;
    let fields = SCHEMA.get_all_fields(type_name);
    let mut parsed = IndexMap::new();
    for (name, value) in object {
        let Some(field) = fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(name))
        else {
            continue;
        };
        parsed.insert(
            name.clone(),
            parse_json_value(value, field, None, rotation_euler)?,
        );
    }
    let compact = match type_name {
        "Vector3" | "HalfVector3" | "ByteVector3" => {
            Some(NifValue::Vec3(array3(&parsed, ["x", "y", "z"])?))
        }
        "Vector4" => Some(NifValue::Vec4(array4(&parsed, ["x", "y", "z", "w"])?)),
        "Color3" => Some(NifValue::Color3(array3(&parsed, ["r", "g", "b"])?)),
        "Color4" | "ByteColor4" | "ByteColor4BGRA" => {
            Some(NifValue::Color4(array4(&parsed, ["r", "g", "b", "a"])?))
        }
        "Quaternion" => Some(NifValue::Quaternion(array4(&parsed, ["w", "x", "y", "z"])?)),
        "Matrix33" => {
            let values = array9(
                &parsed,
                [
                    "m11", "m21", "m31", "m12", "m22", "m32", "m13", "m23", "m33",
                ],
            )?;
            Some(NifValue::Matrix33([
                [values[0], values[1], values[2]],
                [values[3], values[4], values[5]],
                [values[6], values[7], values[8]],
            ]))
        }
        _ => None,
    };
    Ok(compact.unwrap_or(NifValue::Struct(parsed)))
}

fn array3(fields: &IndexMap<String, NifValue>, names: [&str; 3]) -> Result<[f32; 3], String> {
    Ok([
        field_float(fields, names[0])?,
        field_float(fields, names[1])?,
        field_float(fields, names[2])?,
    ])
}

fn array4(fields: &IndexMap<String, NifValue>, names: [&str; 4]) -> Result<[f32; 4], String> {
    Ok([
        field_float(fields, names[0])?,
        field_float(fields, names[1])?,
        field_float(fields, names[2])?,
        field_float(fields, names[3])?,
    ])
}

fn array9(fields: &IndexMap<String, NifValue>, names: [&str; 9]) -> Result<[f32; 9], String> {
    let mut result = [0.0; 9];
    for (index, name) in names.into_iter().enumerate() {
        result[index] = field_float(fields, name)?;
    }
    Ok(result)
}

fn field_float(fields: &IndexMap<String, NifValue>, name: &str) -> Result<f32, String> {
    match fields.get(name) {
        Some(NifValue::Float(value)) => Ok(*value as f32),
        Some(NifValue::Int(value)) => Ok(*value as f32),
        Some(NifValue::UInt(value)) => Ok(*value as f32),
        _ => Err(format!("Missing {name}")),
    }
}

fn field_definition(type_name: &str, key: &str) -> Option<&'static FieldDef> {
    SCHEMA
        .get_all_fields(type_name)
        .into_iter()
        .find(|field| field.name.eq_ignore_ascii_case(bare_name(key)))
}

fn parse_block_key(name: &str) -> Option<(usize, String)> {
    let (index, type_name) = name.split_once(' ')?;
    Some((index.parse().ok()?, type_name.trim().to_string()))
}

fn parse_reference(value: &Value) -> Option<i32> {
    let text = json_text(value)?.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return Some(-1);
    }
    text.split_whitespace().next()?.parse().ok()
}

fn json_text(value: &Value) -> Option<&str> {
    value.as_str()
}

fn json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(|value| parse_i64(value).and_then(|value| u64::try_from(value).ok()))
    })
}

fn parse_i64(value: &str) -> Option<i64> {
    let value = value.trim();
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(
            || value.parse().ok(),
            |value| i64::from_str_radix(value, 16).ok(),
        )
}

fn parse_version(value: &str) -> Option<(u8, u8, u8, u8)> {
    let parts = value
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<u8>, _>>()
        .ok()?;
    Some((
        *parts.first()?,
        *parts.get(1)?,
        *parts.get(2)?,
        *parts.get(3)?,
    ))
}

fn pack_version(version: (u8, u8, u8, u8)) -> u32 {
    ((version.0 as u32) << 24)
        | ((version.1 as u32) << 16)
        | ((version.2 as u32) << 8)
        | version.3 as u32
}

fn infer_game(version: (u8, u8, u8, u8), user_version: u32, bs_version: u32) -> &'static str {
    match (version, user_version, bs_version) {
        ((4, 0, 0, 2), _, _) => "morrowind",
        ((20, 0, 0, 5), _, _) | ((20, 0, 0, 4), _, _) | ((10, _, _, _), _, _) => "oblivion",
        (_, 11, 34) => "fnv",
        (_, 11, _) => "fo3",
        (_, 12, 83) => "skyrim",
        (_, 12, 100) => "skyrimse",
        (_, 12, 155) => "fo76",
        (_, 12, version) if version >= 170 => "starfield",
        _ => "fo4",
    }
}

fn format_float(value: f64, digits: usize) -> String {
    let value = format!("{value:.digits$}");
    let trimmed = value.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn format_edit_float(value: f64, digits: usize) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value.is_infinite() {
        "Inf".to_string()
    } else if value >= f64::from(f32::MAX) {
        "Max".to_string()
    } else if value <= f64::from(f32::MIN) {
        "Min".to_string()
    } else {
        format_float(value, digits)
    }
}

fn bare_name(name: &str) -> &str {
    name.split_once(':').map(|(name, _)| name).unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nif_json_round_trip_preserves_blocks_refs_and_symbolic_values() {
        let mut nif = NifFile::new("skyrimse");
        nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
        nif.blocks[0].set_field("Children", NifValue::Array(vec![NifValue::Ref(1)]));
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
        nif.blocks[0].set_field("Translation", NifValue::Vec3([1.0, 2.0, 3.0]));
        let child = nif.add_block("NiNode", None);
        nif.blocks[child].set_field("Name", NifValue::String("Child".to_string()));

        let json = nif_to_json(&nif, 8, false);
        assert_eq!(json["0 BSFadeNode"]["Children"][0], "1 NiNode \"Child\"");
        assert_eq!(json["0 BSFadeNode"]["Translation"], "1 2 3");
        let mut rebuilt = nif_from_json(&json, false).unwrap();
        assert_eq!(rebuilt.header.bs_version, 100);
        assert_eq!(rebuilt.blocks.len(), 2);
        assert_eq!(
            rebuilt.blocks[0].get_field("Children"),
            Some(&NifValue::Array(vec![NifValue::Ref(1)]))
        );
        assert_eq!(
            rebuilt.blocks[1].get_field("Name"),
            Some(&NifValue::String("Child".to_string()))
        );
        let bytes = rebuilt.to_bytes().unwrap();
        let reparsed = NifFile::from_bytes(&bytes, None).unwrap();
        assert_eq!(reparsed.blocks.len(), 2);
        assert_eq!(
            reparsed.blocks[0].get_field("Children"),
            Some(&NifValue::Array(vec![NifValue::Ref(1)]))
        );
    }

    #[test]
    fn nif_json_uses_hex_colors_and_hex_byte_strings() {
        assert_eq!(
            merged_value_json(
                &NifValue::Color4([1.0, 0.5, 0.0, 1.0]),
                "Color4",
                "Color",
                8,
                false,
            ),
            Some(Value::String("#FF8000FF".to_string()))
        );
        assert_eq!(
            nif_value_json(
                &NifFile::new("fo4"),
                &NifValue::Bytes(vec![0, 10, 255]),
                None,
                "Data",
                8,
                false,
            ),
            Value::String("00 0A FF".to_string())
        );
        assert_eq!(
            parse_merged_value("#FF8000FF", "Color4", false).unwrap(),
            Some(NifValue::Color4([1.0, 128.0 / 255.0, 0.0, 1.0]))
        );
        assert_eq!(parse_hex_bytes("00, 0A; FF").unwrap(), vec![0, 10, 255]);
    }

    #[test]
    fn nif_json_rotation_modes_round_trip() {
        let angle_axis = quaternion_text([0.70710677, 0.0, 0.0, 0.70710677], 8, false);
        let angle_axis_values = split_values(&angle_axis);
        assert!((parse_edit_float(angle_axis_values[0]).unwrap() - 90.0).abs() < 0.0001);
        assert_eq!(&angle_axis_values[1..3], &["0", "0"]);
        assert!((parse_edit_float(angle_axis_values[3]).unwrap() - 1.0).abs() < 0.0001);
        let quaternion = parse_quaternion(&angle_axis, false).unwrap();
        assert!((quaternion[0] - 0.70710677).abs() < 0.00001);
        assert!((quaternion[3] - 0.70710677).abs() < 0.00001);

        let euler = quaternion_text([0.70710677, 0.0, 0.0, 0.70710677], 8, true);
        let euler_values = split_values(&euler);
        assert_eq!(&euler_values[..2], &["0", "0"]);
        assert!((parse_edit_float(euler_values[2]).unwrap() - 90.0).abs() < 0.0001);
        let quaternion = parse_quaternion(&euler, true).unwrap();
        assert!((quaternion[0] - 0.70710677).abs() < 0.00001);
        assert!((quaternion[3] - 0.70710677).abs() < 0.00001);
    }
}
