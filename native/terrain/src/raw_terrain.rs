use crate::btd::{RawTerrainPatch, RAW_COLOR_SIDE, RAW_SELECTOR_SIDE};

pub const MAX_BANK_BYTES: usize = 112 * 1024;
pub const PATCH_BYTES: usize = 16 + RAW_SELECTOR_SIDE * RAW_SELECTOR_SIDE * 2 + 72 + RAW_COLOR_SIDE * RAW_COLOR_SIDE * 2;
pub const CELL_BYTES: usize = 4 * PATCH_BYTES;
/// Bank layout, little-endian:
///
/// ```text
/// u32 magic 'FRBK', u32 byte length (multiple of 4), u32 material count 1..=255
/// count x 72 bytes, in reverse source-ordinal order (palette id n names entry n - 1):
///   u32 source ordinal, u32 source LTEX form id,
///   u32 x 4 offsets of the _d, _n, _l, _r texture paths,
///   f32 x 4 blend row: softness, sin, cos of the rotation, repeats per world unit,
///   f32 x 4 emission row: linear colour, override flag (> 0.1),
///   f32 x 4 exposure row: luminous emittance, exposure offset, min, max
/// NUL-terminated `textures/...` paths, zero-padded to 4 bytes
/// ```
const BANK_HEADER_BYTES: usize = 12;
const BANK_MAGIC: u32 = 0x4B42_5246;
const MATERIAL_BYTES: usize = 72;
const ROWS_OFFSET: usize = 24;
/// Texture maps per material: `_d` diffuse, `_n` normal, `_l` lighting, `_r` reflectance.
pub const MATERIAL_MAPS: usize = 4;
/// Every bank `_r` is a grey F0 map of this edge in `REFLECTANCE_FORMAT` with full mips, so
/// the consumer's `_r` texture array never mixes slice sizes (FO76 ships 4x4 and 256x256).
pub const REFLECTANCE_SIZE: u32 = 256;
pub const REFLECTANCE_FORMAT: &str = "BC4_UNORM";

pub struct MaterialSource {
    pub source_ltex_form_id: u32,
    pub rows: [[f32; 4]; 3],
    pub textures: [String; MATERIAL_MAPS],
}

fn texture_path_valid(path: &str) -> bool {
    (13..=1024).contains(&path.len()) && path.starts_with("textures/") &&
        !path.contains([':', '\\', '\0']) && !path.contains("//") &&
        !path.split('/').any(|part| part == "." || part == "..")
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn bank_path(bytes: &[u8], material: usize, map: usize) -> Result<&str, String> {
    let start = u32_at(bytes, material + 8 + map * 4) as usize;
    if start >= bytes.len() { return Err("raw terrain path offset is invalid".into()); }
    let length = bytes[start..].iter().position(|v| *v == 0).ok_or("raw terrain path is not terminated")?;
    std::str::from_utf8(&bytes[start..start + length]).map_err(|_| "raw terrain path is not UTF-8".into())
}

pub fn validate_bank(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < BANK_HEADER_BYTES || bytes.len() > MAX_BANK_BYTES || bytes.len() % 4 != 0 {
        return Err("raw terrain bank length is invalid".into());
    }
    let count = u32_at(bytes, 8) as usize;
    if u32_at(bytes, 0) != BANK_MAGIC || u32_at(bytes, 4) as usize != bytes.len() ||
        !(1..=255).contains(&count) || BANK_HEADER_BYTES + count * MATERIAL_BYTES >= bytes.len() {
        return Err("raw terrain bank header is invalid".into());
    }
    let strings_start = BANK_HEADER_BYTES + count * MATERIAL_BYTES;
    for index in 0..count {
        let offset = BANK_HEADER_BYTES + index * MATERIAL_BYTES;
        if u32_at(bytes, offset) as usize != count - index - 1 || u32_at(bytes, offset + 4) == 0 {
            return Err("raw terrain bank material identity is invalid".into());
        }
        for value in 0..12 {
            let value = f32::from_bits(u32_at(bytes, offset + ROWS_OFFSET + value * 4));
            if !value.is_finite() { return Err("raw terrain material row is not finite".into()); }
        }
        if f32::from_bits(u32_at(bytes, offset + ROWS_OFFSET)) < 0.0001 { return Err("raw terrain blend softness is invalid".into()); }
        for map in 0..MATERIAL_MAPS {
            if (u32_at(bytes, offset + 8 + map * 4) as usize) < strings_start { return Err("raw terrain path offset is invalid".into()); }
            if !texture_path_valid(bank_path(bytes, offset, map)?) { return Err("raw terrain texture path is invalid".into()); }
        }
    }
    Ok(count)
}

pub fn encode_bank(source_ordinal_order: &[MaterialSource]) -> Result<Vec<u8>, String> {
    let count = source_ordinal_order.len();
    if !(1..=255).contains(&count) { return Err("raw terrain material count must fit the source palette byte".into()); }
    let mut bytes = vec![0u8; BANK_HEADER_BYTES + count * MATERIAL_BYTES];
    bytes[0..4].copy_from_slice(&BANK_MAGIC.to_le_bytes());
    bytes[8..12].copy_from_slice(&(count as u32).to_le_bytes());
    for (index, source) in source_ordinal_order.iter().rev().enumerate() {
        let offset = BANK_HEADER_BYTES + index * MATERIAL_BYTES;
        bytes[offset..offset + 4].copy_from_slice(&((count - index - 1) as u32).to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&source.source_ltex_form_id.to_le_bytes());
        for (value, native) in source.rows.iter().flatten().enumerate() {
            let at = offset + ROWS_OFFSET + value * 4;
            bytes[at..at + 4].copy_from_slice(&native.to_le_bytes());
        }
        for (map, path) in source.textures.iter().enumerate() {
            if !texture_path_valid(path) { return Err("raw terrain texture path is invalid".into()); }
            let path_offset = bytes.len() as u32;
            bytes[offset + 8 + map * 4..offset + 12 + map * 4].copy_from_slice(&path_offset.to_le_bytes());
            bytes.extend_from_slice(path.as_bytes());
            bytes.push(0);
        }
    }
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let length = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    validate_bank(&bytes)?;
    Ok(bytes)
}

/// The bank's materials in table order, the order the runtime indexes them in: palette id `n` names entry
/// `n - 1`. `encode_bank` writes source ordinals reversed, so entry 0 is the last source ordinal.
pub fn decode_bank(bytes: &[u8]) -> Result<Vec<MaterialSource>, String> {
    let count = validate_bank(bytes)?;
    (0..count)
        .map(|index| {
            let offset = BANK_HEADER_BYTES + index * MATERIAL_BYTES;
            let float = |lane: usize| f32::from_bits(u32_at(bytes, offset + ROWS_OFFSET + lane * 4));
            Ok(MaterialSource {
                source_ltex_form_id: u32_at(bytes, offset + 4),
                rows: std::array::from_fn(|row| std::array::from_fn(|lane| float(row * 4 + lane))),
                textures: [0, 1, 2, 3].map(|map| bank_path(bytes, offset, map).map(str::to_owned)).into_iter()
                    .collect::<Result<Vec<_>, String>>()?.try_into().unwrap(),
            })
        })
        .collect()
}

pub fn encode_patch(patch: &RawTerrainPatch) -> Result<Vec<u8>, String> {
    if patch.selectors.len() != RAW_SELECTOR_SIDE * RAW_SELECTOR_SIDE || patch.colors.len() != RAW_COLOR_SIDE * RAW_COLOR_SIDE {
        return Err("raw terrain patch dimensions are invalid".into());
    }
    let mut bytes = Vec::with_capacity(PATCH_BYTES);
    for origin in patch.storage_origin { bytes.extend_from_slice(&origin.to_le_bytes()); }
    for origin in patch.native_grid_origin { bytes.extend_from_slice(&origin.to_le_bytes()); }
    for word in &patch.selectors { bytes.extend_from_slice(&word.to_le_bytes()); }
    for palette in patch.palettes { bytes.extend_from_slice(&palette); }
    for color in &patch.colors { bytes.extend_from_slice(&color.to_le_bytes()); }
    Ok(bytes)
}

pub fn validate_cell(bank: &[u8], bytes: &[u8], cell_x: i32, cell_y: i32) -> Result<(), String> {
    let count = validate_bank(bank)?;
    if bytes.len() != CELL_BYTES { return Err("raw terrain cell length is invalid".into()); }
    let mut native_base = [0.0f32; 2];
    for (quadrant, patch) in bytes.chunks_exact(PATCH_BYTES).enumerate() {
        for axis in 0..2 {
            let quadrant_offset = if axis == 0 { quadrant & 1 } else { quadrant >> 1 } as i64 * 64;
            let cell = if axis == 0 { cell_x } else { cell_y };
            let expected_storage = i64::from(cell) * 128 + 64 + quadrant_offset;
            if i64::from(u32_at(patch, axis * 4) as i32) != expected_storage {
                return Err("raw terrain storage origin does not match cell/quadrant".into());
            }
            let native = f32::from_bits(u32_at(patch, 8 + axis * 4));
            if !native.is_finite() || native % 64.0 != 0.0 { return Err("raw terrain native grid origin is invalid".into()); }
            if quadrant == 0 { native_base[axis] = native; }
            if native != native_base[axis] + quadrant_offset as f32 { return Err("raw terrain native grid origins are discontinuous".into()); }
        }
        let palettes = 16 + RAW_SELECTOR_SIDE * RAW_SELECTOR_SIDE * 2;
        for palette in patch[palettes..palettes + 72].chunks_exact(8) {
            if palette[5] != 0 || palette[7] != 0 || [0,1,2,3,4,6].into_iter().any(|slot| palette[slot] as usize > count) {
                return Err("raw terrain palette is invalid".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn materials(count: u32) -> Vec<MaterialSource> {
        (0..count).map(|i| MaterialSource {
            source_ltex_form_id: 0x1234 + i,
            rows: [[0.25, 0.0, 1.0, -1.0/128.0], [i as f32, 2.0, 3.0, 0.0], [4.0,5.0,6.0,7.0]],
            textures: ["d","n","l","r"].map(|map| format!("textures/terrain/test{i}_source_{map}.dds")),
        }).collect()
    }

    #[test]
    fn raw_bank_reverses_only_material_table_and_preserves_rows_paths() {
        let materials = materials(12);
        let bytes = encode_bank(&materials).unwrap();
        assert_eq!(validate_bank(&bytes).unwrap(), 12);
        assert_eq!(u32_at(&bytes, 4) as usize, bytes.len());
        for index in 0..12 {
            let offset = BANK_HEADER_BYTES + index * MATERIAL_BYTES;
            assert_eq!(u32_at(&bytes, offset), (11-index) as u32);
            assert_eq!(u32_at(&bytes, offset+4), (0x1234+11-index) as u32);
            for (value, expected) in materials[11-index].rows.iter().flatten().enumerate() {
                assert_eq!(u32_at(&bytes, offset+ROWS_OFFSET+value*4), expected.to_bits());
            }
            assert_eq!(bank_path(&bytes, offset, 3).unwrap(), format!("textures/terrain/test{}_source_r.dds", 11 - index));
        }
        // Strings start right after the table.
        assert_eq!(u32_at(&bytes, BANK_HEADER_BYTES + 8) as usize, BANK_HEADER_BYTES + 12 * MATERIAL_BYTES);
        let mut corrupt = bytes.clone();
        corrupt[BANK_HEADER_BYTES + ROWS_OFFSET..BANK_HEADER_BYTES + ROWS_OFFSET + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_bank(&corrupt).is_err());
        let mut corrupt = bytes.clone(); corrupt[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert!(validate_bank(&corrupt).is_err());
        let mut corrupt = bytes.clone(); corrupt[0] ^= 1;
        assert!(validate_bank(&corrupt).is_err());
    }

    #[test]
    fn decode_bank_returns_table_order_with_rows_and_all_four_maps() {
        let decoded = decode_bank(&encode_bank(&materials(3)).unwrap()).unwrap();
        assert_eq!(decoded.iter().map(|m| m.source_ltex_form_id).collect::<Vec<_>>(), [0x1236, 0x1235, 0x1234]);
        assert_eq!(decoded[2].rows, materials(1)[0].rows);
        assert_eq!(decoded[2].textures, ["d","n","l","r"].map(|map| format!("textures/terrain/test0_source_{map}.dds")));
    }

    #[test]
    fn invalid_paths_are_refused() {
        let mut bad = materials(1);
        bad[0].textures[3] = "../escape_r.dds".into();
        assert!(encode_bank(&bad).is_err());
    }
}
