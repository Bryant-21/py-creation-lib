const LAND_VERTEX_COUNT: usize = 33 * 33;
const LAND_GRID_WIDTH: usize = 33;
const HEIGHT_STEP: f32 = 8.0;
const LAND_VERTEX_SPACING: f32 = 128.0;
const MIN_DELTA_UNITS: f32 = -127.0;
const MAX_DELTA_UNITS: f32 = 127.0;

#[derive(Debug, Clone)]
pub struct EncodedVhgt {
    pub offset: f32,
    pub raw: Vec<u8>,
}

pub fn encode_vhgt(heights: &[f32]) -> Result<EncodedVhgt, String> {
    if heights.len() != LAND_VERTEX_COUNT {
        return Err("VHGT requires exactly 33x33 heights".to_string());
    }

    // CK rounds the offset to an integer on save (e.g. 2274.74 → 2275.0,
    // not floor → 2274). Match that so the decoded heights / normals don't
    // drift when CK re-saves.
    let offset = (heights[0] / HEIGHT_STEP).round();
    let offset_height = offset * HEIGHT_STEP;
    let mut raw = Vec::with_capacity(4 + LAND_VERTEX_COUNT + 3);

    raw.extend_from_slice(&offset.to_le_bytes());

    let mut row_start_units = 0.0f32;
    for row in 0..LAND_GRID_WIDTH {
        let mut previous_units = row_start_units;
        for column in 0..LAND_GRID_WIDTH {
            let height = heights[row * LAND_GRID_WIDTH + column];
            let target_units = ((height - offset_height) / HEIGHT_STEP).round();
            let delta = target_units - previous_units;
            // FO4 CK treats -128 deltas as corrupt height data on save; vanilla
            // FO4 LAND uses -127..127 even though the byte is signed.
            if !(MIN_DELTA_UNITS..=MAX_DELTA_UNITS).contains(&delta) {
                return Err("VHGT delta is outside signed i8 range".to_string());
            }

            let delta_i8 = delta as i8;
            raw.push(delta_i8 as u8);
            previous_units += delta_i8 as f32;
            if column == 0 {
                row_start_units = previous_units;
            }
        }
    }

    raw.extend_from_slice(&[0, 0, 0]);

    Ok(EncodedVhgt { offset, raw })
}

pub fn decode_vhgt_heights(encoded: &EncodedVhgt) -> Result<Vec<f32>, String> {
    if encoded.raw.len() != 4 + LAND_VERTEX_COUNT + 3 {
        return Err("VHGT raw data has an invalid length".to_string());
    }

    let offset_units = f32::from_le_bytes(
        encoded.raw[0..4]
            .try_into()
            .map_err(|_| "VHGT offset bytes are invalid".to_string())?,
    );
    let offset_height = offset_units * HEIGHT_STEP;
    let mut heights = Vec::with_capacity(LAND_VERTEX_COUNT);
    let mut row_start_units = 0.0f32;

    for row in 0..LAND_GRID_WIDTH {
        let mut current_units = row_start_units;
        for column in 0..LAND_GRID_WIDTH {
            let index = row * LAND_GRID_WIDTH + column;
            let delta = encoded.raw[4 + index] as i8;
            current_units += delta as f32;
            heights.push(offset_height + current_units * HEIGHT_STEP);
            if column == 0 {
                row_start_units = current_units;
            }
        }
    }

    Ok(heights)
}

pub fn generate_vnml(heights: &[f32]) -> Vec<u8> {
    if heights.len() != LAND_VERTEX_COUNT {
        return Vec::new();
    }

    let quantized: Vec<f32> = heights
        .iter()
        .map(|height| quantize_height(*height))
        .collect();
    let mut normals = Vec::with_capacity(LAND_VERTEX_COUNT * 3);

    for y in 0..LAND_GRID_WIDTH {
        for x in 0..LAND_GRID_WIDTH {
            let left_x = x.saturating_sub(1);
            let right_x = (x + 1).min(LAND_GRID_WIDTH - 1);
            let down_y = y.saturating_sub(1);
            let up_y = (y + 1).min(LAND_GRID_WIDTH - 1);

            let left = quantized[index(left_x, y)];
            let right = quantized[index(right_x, y)];
            let down = quantized[index(x, down_y)];
            let up = quantized[index(x, up_y)];

            let horizontal_x = (right_x - left_x) as f32 * LAND_VERTEX_SPACING;
            let horizontal_y = (up_y - down_y) as f32 * LAND_VERTEX_SPACING;
            let slope_x = (right - left) / horizontal_x;
            let slope_y = (up - down) / horizontal_y;
            let length = (slope_x * slope_x + slope_y * slope_y + 1.0).sqrt();
            let nx = -slope_x / length;
            let ny = -slope_y / length;
            let nz = 1.0 / length;

            normals.push(normal_component(nx));
            normals.push(normal_component(ny));
            normals.push(normal_component(nz));
        }
    }

    normals
}

fn quantize_height(height: f32) -> f32 {
    (height / HEIGHT_STEP).round() * HEIGHT_STEP
}

fn index(x: usize, y: usize) -> usize {
    y * LAND_GRID_WIDTH + x
}

fn normal_component(value: f32) -> u8 {
    // VNML is signed-i8 per axis (value * 127), NOT unsigned-byte centered at
    // 128. CK rewrites these correctly on save (a no-op paint "fixes" cells).
    let clamped = value.clamp(-1.0, 1.0);
    let signed = (clamped * 127.0).round().clamp(-128.0, 127.0) as i8;
    signed as u8
}
