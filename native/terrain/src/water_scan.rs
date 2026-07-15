use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use thiserror::Error;

const FO4_DEFAULT_WATER_OBJECT_ID: u32 = 0x0C8633;
const FO4_CELL_SIZE: f32 = 4096.0;
const VALID_WATER_HEIGHT_LIMIT: f32 = 1.0e8;

#[derive(Debug, Error)]
pub enum WaterScanError {
    #[error("FO76 source worldspace authoring dir was not found: {0}")]
    MissingWorldspaceDir(PathBuf),
    #[error("FO76 source worldspace authoring dir must be under records/WRLD: {0}")]
    InvalidWorldspaceDir(PathBuf),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaterScanOptions {
    pub source_worldspace_authoring_dir: String,
    pub output_manifest_path: String,
    pub source_min_x: i32,
    pub source_min_y: i32,
    pub source_max_x: i32,
    pub source_max_y: i32,
}

#[derive(Debug, Serialize)]
struct WaterManifest {
    default_water_object_id: u32,
    cells: Vec<WaterManifestCell>,
}

#[derive(Debug, Serialize)]
struct WaterManifestCell {
    x: i32,
    y: i32,
    height: f32,
}

#[derive(Debug, Clone, Copy)]
struct ObjectBounds {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

#[derive(Debug, Clone, Copy)]
struct RefWaterCandidate {
    base_id: Option<u32>,
    is_water_base: bool,
    x: Option<f32>,
    y: Option<f32>,
    z: Option<f32>,
    scale: f32,
}

impl Default for RefWaterCandidate {
    fn default() -> Self {
        Self {
            base_id: None,
            is_water_base: false,
            x: None,
            y: None,
            z: None,
            scale: 1.0,
        }
    }
}

pub fn write_water_manifest(options: WaterScanOptions) -> Result<String, WaterScanError> {
    let worldspace_dir = PathBuf::from(&options.source_worldspace_authoring_dir);
    if !worldspace_dir.is_dir() {
        return Err(WaterScanError::MissingWorldspaceDir(worldspace_dir));
    }
    let records_root = records_root_for_worldspace(&worldspace_dir)?;
    let water_bases = collect_water_bases(&records_root)?;
    let mut cells = HashMap::new();

    scan_record_data_files(&worldspace_dir, &mut |record_path| {
        scan_cell_record(&mut cells, record_path, &water_bases, &options)
    })?;

    let mut manifest_cells = cells
        .into_iter()
        .map(|((x, y), height)| WaterManifestCell { x, y, height })
        .collect::<Vec<_>>();
    manifest_cells.sort_by_key(|cell| (cell.x, cell.y));

    let output_path = PathBuf::from(&options.output_manifest_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = WaterManifest {
        default_water_object_id: FO4_DEFAULT_WATER_OBJECT_ID,
        cells: manifest_cells,
    };
    fs::write(&output_path, serde_json::to_string_pretty(&payload)?)?;
    Ok(output_path.to_string_lossy().into_owned())
}

fn records_root_for_worldspace(worldspace_dir: &Path) -> Result<PathBuf, WaterScanError> {
    worldspace_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| WaterScanError::InvalidWorldspaceDir(worldspace_dir.to_path_buf()))
}

fn collect_water_bases(records_root: &Path) -> Result<HashMap<u32, ObjectBounds>, WaterScanError> {
    let mut water_bases = HashMap::new();
    for signature in ["ACTI", "MSTT", "STAT", "PWAT"] {
        let record_dir = records_root.join(signature);
        if !record_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(record_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }
            if let Some((object_id, bounds)) = scan_water_base(&path)? {
                water_bases.insert(object_id, bounds);
            }
        }
    }
    Ok(water_bases)
}

fn scan_water_base(path: &Path) -> Result<Option<(u32, ObjectBounds)>, WaterScanError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut form_id = form_id_from_filename(path);
    let mut is_water_base = false;
    let mut x1 = 0;
    let mut y1 = 0;
    let mut x2 = 0;
    let mut y2 = 0;

    for line in reader.lines() {
        let line = line?;
        if let Some((key, value)) = line_key_value(&line) {
            match key.as_str() {
                "form_id" => {
                    if form_id.is_none() {
                        form_id = object_id(&value);
                    }
                }
                "WaterType" => is_water_base = true,
                "MODL" | "Model" => {
                    if is_water_model_path(&value) {
                        is_water_base = true;
                    }
                }
                "ObjectBoundsX1" => x1 = int_value(&value).unwrap_or(0),
                "ObjectBoundsY1" => y1 = int_value(&value).unwrap_or(0),
                "ObjectBoundsX2" => x2 = int_value(&value).unwrap_or(0),
                "ObjectBoundsY2" => y2 = int_value(&value).unwrap_or(0),
                _ => {}
            }
        }
    }

    if !is_water_base {
        return Ok(None);
    }
    let Some(object_id) = form_id else {
        return Ok(None);
    };
    Ok(Some((
        object_id,
        ObjectBounds {
            min_x: x1.min(x2),
            min_y: y1.min(y2),
            max_x: x1.max(x2),
            max_y: y1.max(y2),
        },
    )))
}

fn scan_record_data_files(
    dir: &Path,
    scan: &mut impl FnMut(&Path) -> Result<(), WaterScanError>,
) -> Result<(), WaterScanError> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            scan_record_data_files(&path, scan)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("RecordData.yaml") {
            scan(&path)?;
        }
    }
    Ok(())
}

fn scan_cell_record(
    cells: &mut HashMap<(i32, i32), f32>,
    record_path: &Path,
    water_bases: &HashMap<u32, ObjectBounds>,
    options: &WaterScanOptions,
) -> Result<(), WaterScanError> {
    let file = fs::File::open(record_path)?;
    let reader = BufReader::new(file);
    let mut cell_x = None;
    let mut cell_y = None;
    let mut water_height = None;
    let mut in_ref_group = false;
    let mut current_ref = RefWaterCandidate::default();
    let mut expect_base_object_id = false;

    for line in reader.lines() {
        let line = line?;
        if line.trim_start().starts_with("raw_payload_hex:") {
            continue;
        }
        let stripped = line.trim();
        let key_value = line_key_value(&line);
        if let Some((key, value)) = &key_value {
            match key.as_str() {
                "X" if cell_x.is_none() => cell_x = int_value(value),
                "Y" if cell_y.is_none() => cell_y = int_value(value),
                "WaterHeight" => water_height = float_value(value),
                _ => {}
            }
        }

        if matches!(
            stripped,
            "Temporary:" | "Persistent:" | "VisibleWhenDistant:"
        ) {
            if in_ref_group {
                add_ref_water_height(cells, current_ref, water_bases, options);
            }
            in_ref_group = true;
            current_ref = RefWaterCandidate::default();
            expect_base_object_id = false;
            continue;
        }
        if in_ref_group && is_top_level_line(&line) {
            add_ref_water_height(cells, current_ref, water_bases, options);
            in_ref_group = false;
            current_ref = RefWaterCandidate::default();
            expect_base_object_id = false;
            continue;
        }
        if !in_ref_group {
            continue;
        }

        if stripped.starts_with("- form_id:") {
            add_ref_water_height(cells, current_ref, water_bases, options);
            current_ref = RefWaterCandidate::default();
            expect_base_object_id = false;
            continue;
        }
        if stripped == "- Base:" {
            expect_base_object_id = true;
            continue;
        }
        if stripped.starts_with("- ") && stripped != "- Base:" {
            expect_base_object_id = false;
        }

        if let Some((key, value)) = key_value {
            if key == "object_id" && expect_base_object_id {
                current_ref.base_id = object_id(&value);
                current_ref.is_water_base = current_ref
                    .base_id
                    .is_some_and(|base_id| water_bases.contains_key(&base_id));
                expect_base_object_id = false;
            } else if !current_ref.is_water_base {
                continue;
            } else {
                match key.as_str() {
                    "Scale" => current_ref.scale = float_value(&value).unwrap_or(1.0),
                    "PositionRotationPositionX" => current_ref.x = float_value(&value),
                    "PositionRotationPositionY" => current_ref.y = float_value(&value),
                    "PositionRotationPositionZ" => current_ref.z = float_value(&value),
                    _ => {}
                }
            }
        }
    }

    if in_ref_group {
        add_ref_water_height(cells, current_ref, water_bases, options);
    }
    if let (Some(x), Some(y), Some(height)) = (cell_x, cell_y, water_height) {
        if is_valid_water_height(height) && range_contains(x, y, options) {
            set_max_height(cells, x, y, height);
        }
    }
    Ok(())
}

fn add_ref_water_height(
    cells: &mut HashMap<(i32, i32), f32>,
    ref_water: RefWaterCandidate,
    water_bases: &HashMap<u32, ObjectBounds>,
    options: &WaterScanOptions,
) {
    let Some(base_id) = ref_water.base_id else {
        return;
    };
    let Some(bounds) = water_bases.get(&base_id) else {
        return;
    };
    let (Some(x), Some(y), Some(height)) = (ref_water.x, ref_water.y, ref_water.z) else {
        return;
    };
    if !is_valid_water_height(height) {
        return;
    }
    for (cell_x, cell_y) in cells_touched_by_bounds(x, y, *bounds, ref_water.scale) {
        if range_contains(cell_x, cell_y, options) {
            set_max_height(cells, cell_x, cell_y, height);
        }
    }
}

fn cells_touched_by_bounds(
    x: f32,
    y: f32,
    bounds: ObjectBounds,
    scale: f32,
) -> impl Iterator<Item = (i32, i32)> {
    let min_cell_x = ((x + bounds.min_x as f32 * scale) / FO4_CELL_SIZE).floor() as i32;
    let max_cell_x = ((x + bounds.max_x as f32 * scale) / FO4_CELL_SIZE).floor() as i32;
    let min_cell_y = ((y + bounds.min_y as f32 * scale) / FO4_CELL_SIZE).floor() as i32;
    let max_cell_y = ((y + bounds.max_y as f32 * scale) / FO4_CELL_SIZE).floor() as i32;
    let min_x = min_cell_x.min(max_cell_x);
    let max_x = min_cell_x.max(max_cell_x);
    let min_y = min_cell_y.min(max_cell_y);
    let max_y = min_cell_y.max(max_cell_y);
    (min_y..=max_y).flat_map(move |cell_y| (min_x..=max_x).map(move |cell_x| (cell_x, cell_y)))
}

fn is_top_level_line(line: &str) -> bool {
    !line.is_empty() && !line.starts_with(' ') && !line.starts_with('-')
}

fn is_water_model_path(value: &str) -> bool {
    let normalized = value.replace('/', "\\").to_ascii_lowercase();
    normalized.starts_with("water\\") || normalized.contains("\\water\\")
}

fn line_key_value(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    let trimmed = trimmed
        .strip_prefix("- ")
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let colon = trimmed.find(':')?;
    let key = trimmed[..colon].trim().trim_matches(['"', '\'']);
    if key.is_empty() {
        return None;
    }
    let value = trimmed[colon + 1..].trim().trim_matches(['"', '\'']);
    Some((key.to_string(), value.to_string()))
}

fn range_contains(cell_x: i32, cell_y: i32, options: &WaterScanOptions) -> bool {
    if options.source_max_x < options.source_min_x || options.source_max_y < options.source_min_y {
        return true;
    }
    options.source_min_x <= cell_x
        && cell_x <= options.source_max_x
        && options.source_min_y <= cell_y
        && cell_y <= options.source_max_y
}

fn set_max_height(cells: &mut HashMap<(i32, i32), f32>, x: i32, y: i32, height: f32) {
    let entry = cells.entry((x, y)).or_insert(height);
    *entry = entry.max(height);
}

fn is_valid_water_height(value: f32) -> bool {
    value.is_finite() && value.abs() < VALID_WATER_HEIGHT_LIMIT
}

fn form_id_from_filename(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".yaml").unwrap_or(name);
    let id_segment = stem.rsplit(" - ").next().unwrap_or(stem);
    let candidate = id_segment.split('_').next()?.trim();
    if candidate.len() != 6 || !candidate.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    object_id(candidate)
}

fn object_id(value: &str) -> Option<u32> {
    let mut text = value.split(':').next().unwrap_or(value).trim();
    text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    u32::from_str_radix(text, 16)
        .ok()
        .map(|parsed| parsed & 0x00FF_FFFF)
}

fn float_value(value: &str) -> Option<f32> {
    value.parse::<f32>().ok()
}

fn int_value(value: &str) -> Option<i32> {
    value.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn water_manifest_uses_placed_water_ref_height() {
        let temp_dir = unique_temp_dir("water_scan_ref");
        let records_root = temp_dir.join("records");
        let acti_dir = records_root.join("ACTI");
        let worldspace_dir = records_root
            .join("WRLD")
            .join("APPALACHIA - 25DA15_SeventySix.esm");
        let cell_dir = worldspace_dir
            .join("0, 0")
            .join("0, 0")
            .join("000001_SeventySix.esm");
        fs::create_dir_all(&acti_dir).expect("acti dir");
        fs::create_dir_all(&cell_dir).expect("cell dir");
        fs::write(
            acti_dir.join("WaterAngle45ExtClear - 52ACBF_SeventySix.esm.yaml"),
            r#"form_id: 52ACBF
eid: WaterAngle45ExtClear
fields:
- ObjectBounds:
    ObjectBoundsX1: -8
    ObjectBoundsY1: -8
    ObjectBoundsX2: 5000
    ObjectBoundsY2: 8
- MODL: Water\WaterAngle45.nif
- WaterType:
    reference:
      plugin: SeventySix.esm
      object_id: 0C8633
"#,
        )
        .expect("water base");
        fs::write(
            acti_dir.join("Rock01 - 000123_SeventySix.esm.yaml"),
            r#"form_id: "000123"
eid: Rock01
fields:
- MODL: Rocks\Rock01.nif
"#,
        )
        .expect("non-water base");
        fs::write(
            cell_dir.join("RecordData.yaml"),
            r#"form_id: "000001"
fields:
- DATA:
    raw_hex: "02000000"
- Grid:
    X: 0
    "Y": 0
- WaterHeight: 3.4028234663852886e+38
Temporary:
- form_id: "000100"
  fields:
  - Base:
      reference:
        plugin: SeventySix.esm
        object_id: 52ACBF
  - DATA:
      PositionRotationPositionX: 123.0
      PositionRotationPositionY: 456.0
      PositionRotationPositionZ: 1600.0
  signature: REFR
- form_id: "000101"
  fields:
  - Base:
      reference:
        plugin: SeventySix.esm
        object_id: "000123"
  - DATA:
      PositionRotationPositionX: 300.0
      PositionRotationPositionY: 400.0
      PositionRotationPositionZ: 9000.0
  signature: REFR
"#,
        )
        .expect("cell yaml");

        let output_path = temp_dir.join("water_manifest.json");
        let result = write_water_manifest(WaterScanOptions {
            source_worldspace_authoring_dir: worldspace_dir.to_string_lossy().into_owned(),
            output_manifest_path: output_path.to_string_lossy().into_owned(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 1,
            source_max_y: 0,
        })
        .expect("water manifest");
        let payload: Value =
            serde_json::from_str(&fs::read_to_string(&output_path).expect("manifest text"))
                .expect("manifest json");
        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result, output_path.to_string_lossy());
        assert_eq!(payload["default_water_object_id"], 0x0C8633);
        assert_eq!(
            payload["cells"],
            serde_json::json!([
                {"x": 0, "y": 0, "height": 1600.0},
                {"x": 1, "y": 0, "height": 1600.0}
            ])
        );
    }

    #[test]
    fn water_manifest_uses_finite_cell_water_height() {
        let temp_dir = unique_temp_dir("water_scan_cell_height");
        let worldspace_dir = temp_dir
            .join("records")
            .join("WRLD")
            .join("APPALACHIA - 25DA15_SeventySix.esm");
        let cell_dir = worldspace_dir
            .join("0, 0")
            .join("0, 0")
            .join("000001_SeventySix.esm");
        fs::create_dir_all(&cell_dir).expect("cell dir");
        fs::write(
            cell_dir.join("RecordData.yaml"),
            r#"form_id: "000001"
fields:
- Grid:
    X: -57
    "Y": 30
- WaterHeight: 0.0
"#,
        )
        .expect("cell yaml");

        let output_path = temp_dir.join("water_manifest.json");
        write_water_manifest(WaterScanOptions {
            source_worldspace_authoring_dir: worldspace_dir.to_string_lossy().into_owned(),
            output_manifest_path: output_path.to_string_lossy().into_owned(),
            source_min_x: -58,
            source_min_y: 29,
            source_max_x: -56,
            source_max_y: 31,
        })
        .expect("water manifest");
        let payload: Value =
            serde_json::from_str(&fs::read_to_string(&output_path).expect("manifest text"))
                .expect("manifest json");
        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            payload["cells"],
            serde_json::json!([{"x": -57, "y": 30, "height": 0.0}])
        );
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("terrain_native_{name}_{nanos}"))
    }
}
