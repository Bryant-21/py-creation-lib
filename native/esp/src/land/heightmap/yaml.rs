use super::parser::LandError;
use super::types::{LandHeightMap, LandVertexNormals};
use serde_json::{Map, Value, json};

const GRID: usize = 33;

fn err(message: impl Into<String>) -> LandError {
    LandError::Other(message.into())
}

fn as_object<'a>(value: &'a Value, key: &str) -> Result<&'a Map<String, Value>, LandError> {
    value
        .as_object()
        .ok_or_else(|| err(format!("expected object for {key}")))
}

fn as_array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, LandError> {
    value
        .as_array()
        .ok_or_else(|| err(format!("expected array for {key}")))
}

fn get<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a Value, LandError> {
    map.get(key)
        .ok_or_else(|| err(format!("missing field: {key}")))
}

fn parse_f32(value: &Value, key: &str) -> Result<f32, LandError> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| err(format!("{key} must be float")))
}

fn parse_i8(value: &Value, key: &str) -> Result<i8, LandError> {
    value
        .as_i64()
        .and_then(|v| i8::try_from(v).ok())
        .ok_or_else(|| err(format!("{key} must be i8")))
}

pub fn heightmap_to_yaml(map: &LandHeightMap) -> Value {
    let mut obj = Map::new();
    obj.insert("base".into(), json!(map.base));
    let rows: Vec<Value> = map
        .deltas
        .iter()
        .map(|row| Value::Array(row.iter().map(|v| json!(*v)).collect()))
        .collect();
    obj.insert("deltas".into(), Value::Array(rows));
    Value::Object(obj)
}

pub fn heightmap_from_yaml(value: &Value) -> Result<LandHeightMap, LandError> {
    let obj = as_object(value, "heightmap")?;
    let base = parse_f32(get(obj, "base")?, "base")?;
    let rows = as_array(get(obj, "deltas")?, "deltas")?;
    if rows.len() != GRID {
        return Err(err(format!(
            "deltas must be {GRID} rows; got {}",
            rows.len()
        )));
    }
    let mut deltas = [[0i8; GRID]; GRID];
    for (r, row_value) in rows.iter().enumerate() {
        let cols = as_array(row_value, &format!("deltas[{r}]"))?;
        if cols.len() != GRID {
            return Err(err(format!(
                "deltas[{r}] must be {GRID} entries; got {}",
                cols.len()
            )));
        }
        for (c, entry) in cols.iter().enumerate() {
            deltas[r][c] = parse_i8(entry, &format!("deltas[{r}][{c}]"))?;
        }
    }
    Ok(LandHeightMap { base, deltas })
}

pub fn vertex_normals_to_yaml(n: &LandVertexNormals) -> Value {
    let mut obj = Map::new();
    let rows: Vec<Value> = n
        .normals
        .iter()
        .map(|row| Value::Array(row.iter().map(|(x, y, z)| json!([*x, *y, *z])).collect()))
        .collect();
    obj.insert("normals".into(), Value::Array(rows));
    Value::Object(obj)
}

pub fn vertex_normals_from_yaml(value: &Value) -> Result<LandVertexNormals, LandError> {
    let obj = as_object(value, "vertex_normals")?;
    let rows = as_array(get(obj, "normals")?, "normals")?;
    if rows.len() != GRID {
        return Err(err(format!(
            "normals must be {GRID} rows; got {}",
            rows.len()
        )));
    }
    let mut normals = [[(0i8, 0i8, 0i8); GRID]; GRID];
    for (r, row_value) in rows.iter().enumerate() {
        let cols = as_array(row_value, &format!("normals[{r}]"))?;
        if cols.len() != GRID {
            return Err(err(format!(
                "normals[{r}] must be {GRID} entries; got {}",
                cols.len()
            )));
        }
        for (c, entry) in cols.iter().enumerate() {
            let triple = as_array(entry, &format!("normals[{r}][{c}]"))?;
            if triple.len() != 3 {
                return Err(err(format!(
                    "normals[{r}][{c}] must be [x, y, z]; got {}",
                    triple.len()
                )));
            }
            normals[r][c] = (
                parse_i8(&triple[0], &format!("normals[{r}][{c}].x"))?,
                parse_i8(&triple[1], &format!("normals[{r}][{c}].y"))?,
                parse_i8(&triple[2], &format!("normals[{r}][{c}].z"))?,
            );
        }
    }
    Ok(LandVertexNormals { normals })
}
