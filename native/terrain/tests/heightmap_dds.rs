use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use terrain_native::heightmap_dds::{write_grayscale_bmp, write_r32_float_dds};

#[test]
fn write_r32_float_dds_matches_ck_exported_r32f_header() {
    let path = unique_temp_path("heightmap_r32f_test.dds");
    write_r32_float_dds(&path, 2, 2, &[1.0, 2.0, 3.5, -4.0]).unwrap();

    let bytes = fs::read(&path).unwrap();
    let _ = fs::remove_file(&path);

    assert_eq!(&bytes[0..4], b"DDS ");
    assert_eq!(u32_at(&bytes, 4), 124);
    assert_eq!(u32_at(&bytes, 8), 0x0002100F);
    assert_eq!(u32_at(&bytes, 12), 2);
    assert_eq!(u32_at(&bytes, 16), 2);
    assert_eq!(u32_at(&bytes, 20), 8);
    assert_eq!(u32_at(&bytes, 24), 1);
    assert_eq!(u32_at(&bytes, 28), 1);
    assert_eq!(&bytes[32..76], &[0; 44]);
    assert_eq!(u32_at(&bytes, 76), 32);
    assert_eq!(u32_at(&bytes, 80), 0x4);
    assert_eq!(u32_at(&bytes, 84), 0x72);
    assert_eq!(u32_at(&bytes, 88), 0);
    assert_eq!(u32_at(&bytes, 108), 0x1000);
    assert_eq!(bytes.len(), 128 + 16);
    assert_eq!(f32_at(&bytes, 128), 1.0);
    assert_eq!(f32_at(&bytes, 132), 2.0);
    assert_eq!(f32_at(&bytes, 136), 3.5);
    assert_eq!(f32_at(&bytes, 140), -4.0);
}

#[test]
fn write_grayscale_bmp_emits_viewable_normalized_preview() {
    let path = unique_temp_path("heightmap_preview_test.bmp");
    write_grayscale_bmp(&path, 2, 2, &[10.0, 20.0, 30.0, 40.0]).unwrap();

    let bytes = fs::read(&path).unwrap();
    let _ = fs::remove_file(&path);

    assert_eq!(&bytes[0..2], b"BM");
    assert_eq!(u32_at(&bytes, 10), 14 + 40 + 256 * 4);
    assert_eq!(i32_at(&bytes, 18), 2);
    assert_eq!(i32_at(&bytes, 22), 2);
    assert_eq!(u16_at(&bytes, 28), 8);
    assert_eq!(bytes.len(), 14 + 40 + 256 * 4 + 8);

    let pixels = &bytes[14 + 40 + 256 * 4..];
    assert_eq!(pixels, &[170, 255, 0, 0, 0, 85, 0, 0]);
}

fn unique_temp_path(file_name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "terrain_native_{}_{}_{}",
        std::process::id(),
        nanos,
        file_name
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
