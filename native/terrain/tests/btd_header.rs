use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use terrain_native::btd::BtdFile;

fn appalachia_btd() -> Option<PathBuf> {
    let data = env::var_os("FO76_DATA")?;
    let path = PathBuf::from(data).join("Terrain").join("Appalachia.btd");
    path.is_file().then_some(path)
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_temp_btd(bytes: Vec<u8>, label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = env::temp_dir().join(format!("{label}_{unique}.btd"));
    fs::write(&path, bytes).unwrap();
    path
}

fn minimal_one_cell_btd(ltex_count: u32, gcvr_count: u32) -> Vec<u8> {
    let cell_count = 1usize;
    let ltex_offset = 0x2cusize;
    let cell_height_minmax_offset = ltex_offset + ltex_count as usize * 4;
    let ltex_map_offset = cell_height_minmax_offset + cell_count * 8;
    let gcvr_count_offset = ltex_map_offset + cell_count * 32;
    let gcvr_offset = gcvr_count_offset + 4;
    let gcvr_map_offset = gcvr_offset + gcvr_count as usize * 4;
    let height_lod4_offset = gcvr_map_offset + cell_count * 32;
    let land_texture_lod4_offset = height_lod4_offset + cell_count * 128;
    let vertex_color_lod4_offset = land_texture_lod4_offset + cell_count * 128;
    let zlib_table_offset = vertex_color_lod4_offset + cell_count * 128;
    let zlib_lod2_offset = zlib_table_offset + 2 * 8;
    let zlib_lod1_offset = zlib_lod2_offset + 2 * 8;
    let zlib_lod0_offset = zlib_lod1_offset + 1 * 8;
    let zlib_data_offset = zlib_lod0_offset + 2 * 8;
    let mut bytes = vec![0u8; zlib_data_offset];

    bytes[0..4].copy_from_slice(b"BTDB");
    put_u32(&mut bytes, 0x04, 6);
    put_u32(&mut bytes, 0x10, 128);
    put_u32(&mut bytes, 0x14, 128);
    put_i32(&mut bytes, 0x18, 0);
    put_i32(&mut bytes, 0x1c, 0);
    put_i32(&mut bytes, 0x20, 0);
    put_i32(&mut bytes, 0x24, 0);
    put_u32(&mut bytes, 0x28, ltex_count);
    put_u32(&mut bytes, gcvr_count_offset, gcvr_count);
    bytes
}

#[test]
fn parse_real_appalachia_header_when_available() {
    let Some(path) = appalachia_btd() else {
        eprintln!("skipping: FO76_DATA/Terrain/Appalachia.btd is not available");
        return;
    };

    let btd = BtdFile::open_header(path.to_str().unwrap()).unwrap();
    let report = btd.to_report();

    assert_eq!(report.magic, "BTDB");
    assert_eq!(report.version, 6);
    assert_eq!(report.resolution_x, 25728);
    assert_eq!(report.resolution_y, 25728);
    assert_eq!(report.cell_min_x, -100);
    assert_eq!(report.cell_min_y, -100);
    assert_eq!(report.cell_max_x, 100);
    assert_eq!(report.cell_max_y, 100);
    assert_eq!(report.ltex_count, 43);
}

#[test]
fn decodes_gcvr_table_and_quadrant_ground_cover_slots() {
    let mut bytes = minimal_one_cell_btd(3, 2);
    let ltex_offset = 0x2cusize;
    let cell_height_minmax_offset = ltex_offset + 3 * 4;
    let ltex_map_offset = cell_height_minmax_offset + 8;
    let gcvr_count_offset = ltex_map_offset + 32;
    let gcvr_offset = gcvr_count_offset + 4;
    let gcvr_map_offset = gcvr_offset + 2 * 4;

    put_u32(&mut bytes, ltex_offset, 0xFF00_0010);
    put_u32(&mut bytes, ltex_offset + 4, 0xFF00_0020);
    put_u32(&mut bytes, ltex_offset + 8, 0xFF00_0030);
    put_u32(&mut bytes, gcvr_offset, 0xFF00_00A0);
    put_u32(&mut bytes, gcvr_offset + 4, 0xFF00_00B0);

    bytes[gcvr_map_offset..gcvr_map_offset + 32].fill(0xFF);
    bytes[ltex_map_offset] = 1;
    bytes[ltex_map_offset + 4] = 2;
    bytes[ltex_map_offset + 6] = 3;
    bytes[gcvr_map_offset] = 1;
    bytes[gcvr_map_offset + 4] = 0;
    bytes[gcvr_map_offset + 6] = 1;

    let path = write_temp_btd(bytes, "gcvr_slots");
    let btd = BtdFile::open(path.to_str().unwrap()).unwrap();
    let _ = fs::remove_file(path);

    assert_eq!(btd.land_texture_form_id(0), Some(0xFF00_0010));
    assert_eq!(btd.ground_cover_form_id(0), Some(0xFF00_00A0));
    assert_eq!(btd.ground_cover_form_id(1), Some(0xFF00_00B0));

    let set = btd.cell_texture_set(0, 0).unwrap();
    let quadrant = &set.quadrants[0];
    assert_eq!(quadrant.base, Some(0));
    assert_eq!(quadrant.base_source_slot, Some(6));
    assert_eq!(quadrant.additional[0], Some(1));
    assert_eq!(quadrant.additional_source_slots[0], Some(4));
    assert_eq!(quadrant.additional[4], Some(2));
    assert_eq!(quadrant.additional_source_slots[4], Some(0));
    assert_eq!(quadrant.ground_cover[6], Some(1));
    assert_eq!(quadrant.ground_cover[4], Some(0));
    assert_eq!(quadrant.ground_cover[0], Some(1));
}

#[test]
fn extract_real_lod0_cell_when_available() {
    let Some(path) = appalachia_btd() else {
        eprintln!("skipping: FO76_DATA/Terrain/Appalachia.btd is not available");
        return;
    };

    let mut btd = BtdFile::open(path.to_str().unwrap()).unwrap();
    let heights = btd.cell_height_map_u16(0, 0, 0).unwrap();
    let alphas = btd.cell_land_alpha_u16(0, 0, 0).unwrap();
    let texture_set = btd.cell_texture_set(0, 0).unwrap();

    assert_eq!(heights.len(), 128 * 128);
    assert_eq!(alphas.len(), 128 * 128);
    assert_eq!(texture_set.quadrants.len(), 4);
    assert!(heights.iter().any(|value| *value != heights[0]));
    assert!(alphas.iter().any(|value| *value != alphas[0]));
    assert!(
        texture_set
            .quadrants
            .iter()
            .any(|quadrant| quadrant.base.is_some())
    );
}

#[test]
fn extract_real_off_origin_cell_and_reject_bad_requests_when_available() {
    let Some(path) = appalachia_btd() else {
        eprintln!("skipping: FO76_DATA/Terrain/Appalachia.btd is not available");
        return;
    };

    let mut btd = BtdFile::open(path.to_str().unwrap()).unwrap();
    let heights = btd.cell_height_map_u16(-1, -1, 0).unwrap();
    let alphas = btd.cell_land_alpha_u16(-1, -1, 0).unwrap();
    let lod2 = btd.cell_height_map_u16(-1, -1, 2).unwrap();

    assert_eq!(heights.len(), 128 * 128);
    assert_eq!(alphas.len(), 128 * 128);
    assert_eq!(lod2.len(), 32 * 32);
    assert!(btd.cell_height_map_u16(-1, -1, 5).is_err());
    assert!(btd.cell_height_map_u16(-101, -101, 0).is_err());
}

#[test]
fn malformed_header_with_oversized_spans_is_rejected() {
    let mut bytes = vec![0u8; 0x2c];
    bytes[0..4].copy_from_slice(b"BTDB");
    put_u32(&mut bytes, 0x04, 6);
    put_u32(&mut bytes, 0x10, 25728);
    put_u32(&mut bytes, 0x14, 25728);
    put_i32(&mut bytes, 0x18, i32::MIN);
    put_i32(&mut bytes, 0x1c, i32::MIN);
    put_i32(&mut bytes, 0x20, i32::MAX);
    put_i32(&mut bytes, 0x24, i32::MAX);

    let path = write_temp_btd(bytes, "malformed_btd");

    let result = BtdFile::open_header(path.to_str().unwrap());
    let _ = fs::remove_file(path);

    assert!(result.is_err());
}
