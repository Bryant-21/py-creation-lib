use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

const DDS_MAGIC: &[u8; 4] = b"DDS ";
const DDS_HEADER_SIZE: u32 = 124;
const DDS_PIXEL_FORMAT_SIZE: u32 = 32;
const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PITCH: u32 = 0x8;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x20000;
const DDPF_FOURCC: u32 = 0x4;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const D3DFMT_R32F: u32 = 0x72;

#[derive(Debug, thiserror::Error)]
pub enum HeightmapDdsError {
    #[error("file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Message(String),
}

pub fn write_r32_float_dds(
    path: &Path,
    width: usize,
    height: usize,
    values: &[f32],
) -> Result<(), HeightmapDdsError> {
    let expected_len = checked_pixel_count(width, height)?;
    if values.len() != expected_len {
        return Err(HeightmapDdsError::Message(
            "heightmap values length does not match dimensions".to_string(),
        ));
    }

    let file = create_output_file(path)?;
    let mut writer = BufWriter::new(file);
    write_r32_float_dds_header(&mut writer, width, height)?;
    write_f32_values(&mut writer, values)?;
    writer.flush()?;
    Ok(())
}

pub fn write_grayscale_bmp(
    path: &Path,
    width: usize,
    height: usize,
    values: &[f32],
) -> Result<(), HeightmapDdsError> {
    let expected_len = checked_pixel_count(width, height)?;
    if values.len() != expected_len {
        return Err(HeightmapDdsError::Message(
            "heightmap values length does not match dimensions".to_string(),
        ));
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(HeightmapDdsError::Message(
            "heightmap values must be finite for BMP preview".to_string(),
        ));
    }

    let width_u32 = u32::try_from(width)
        .map_err(|_| HeightmapDdsError::Message("BMP width exceeds u32".to_string()))?;
    let height_u32 = u32::try_from(height)
        .map_err(|_| HeightmapDdsError::Message("BMP height exceeds u32".to_string()))?;
    let row_stride = usize::try_from(width_u32)
        .ok()
        .and_then(|value| value.checked_add(3))
        .map(|value| value & !3)
        .ok_or_else(|| HeightmapDdsError::Message("BMP row stride overflow".to_string()))?;
    let image_size = checked_mul(row_stride, height, "BMP image size overflow")?;
    let palette_size = 256usize
        .checked_mul(4)
        .ok_or_else(|| HeightmapDdsError::Message("BMP palette size overflow".to_string()))?;
    let pixel_offset = 14usize
        .checked_add(40)
        .and_then(|value| value.checked_add(palette_size))
        .ok_or_else(|| HeightmapDdsError::Message("BMP pixel offset overflow".to_string()))?;
    let file_size = pixel_offset
        .checked_add(image_size)
        .ok_or_else(|| HeightmapDdsError::Message("BMP file size overflow".to_string()))?;

    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let range = max - min;

    let file = create_output_file(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"BM")?;
    write_u32_checked(&mut writer, file_size, "BMP file size exceeds u32")?;
    write_u16(&mut writer, 0)?;
    write_u16(&mut writer, 0)?;
    write_u32_checked(&mut writer, pixel_offset, "BMP pixel offset exceeds u32")?;

    write_u32(&mut writer, 40)?;
    write_i32_checked(&mut writer, width_u32, "BMP width exceeds i32")?;
    write_i32_checked(&mut writer, height_u32, "BMP height exceeds i32")?;
    write_u16(&mut writer, 1)?;
    write_u16(&mut writer, 8)?;
    write_u32(&mut writer, 0)?;
    write_u32_checked(&mut writer, image_size, "BMP image size exceeds u32")?;
    write_i32(&mut writer, 0)?;
    write_i32(&mut writer, 0)?;
    write_u32(&mut writer, 256)?;
    write_u32(&mut writer, 256)?;

    for value in 0..=255u8 {
        writer.write_all(&[value, value, value, 0])?;
    }

    let padding = vec![0u8; row_stride - width];
    for y in (0..height).rev() {
        let row = &values[y * width..(y + 1) * width];
        for value in row {
            let pixel = if range > 0.0 {
                (((*value - min) / range) * 255.0).round().clamp(0.0, 255.0) as u8
            } else {
                0
            };
            writer.write_all(&[pixel])?;
        }
        writer.write_all(&padding)?;
    }

    writer.flush()?;
    Ok(())
}

fn create_output_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    File::create(path)
}

fn write_r32_float_dds_header(
    writer: &mut impl Write,
    width: usize,
    height: usize,
) -> Result<(), HeightmapDdsError> {
    let width = u32::try_from(width)
        .map_err(|_| HeightmapDdsError::Message("heightmap width exceeds u32".to_string()))?;
    let height = u32::try_from(height)
        .map_err(|_| HeightmapDdsError::Message("heightmap height exceeds u32".to_string()))?;
    let pitch = width
        .checked_mul(4)
        .ok_or_else(|| HeightmapDdsError::Message("heightmap pitch overflow".to_string()))?;

    writer.write_all(DDS_MAGIC)?;
    write_u32(writer, DDS_HEADER_SIZE)?;
    write_u32(
        writer,
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_PIXELFORMAT | DDSD_MIPMAPCOUNT,
    )?;
    write_u32(writer, height)?;
    write_u32(writer, width)?;
    write_u32(writer, pitch)?;
    write_u32(writer, 1)?;
    write_u32(writer, 1)?;
    for _ in 0..11 {
        write_u32(writer, 0)?;
    }

    write_u32(writer, DDS_PIXEL_FORMAT_SIZE)?;
    write_u32(writer, DDPF_FOURCC)?;
    write_u32(writer, D3DFMT_R32F)?;
    for _ in 0..5 {
        write_u32(writer, 0)?;
    }

    write_u32(writer, DDSCAPS_TEXTURE)?;
    write_u32(writer, 0)?;
    write_u32(writer, 0)?;
    write_u32(writer, 0)?;
    write_u32(writer, 0)?;
    Ok(())
}

fn write_f32_values(writer: &mut impl Write, values: &[f32]) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    writer.write_all(&bytes)
}

fn write_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32_checked(
    writer: &mut impl Write,
    value: usize,
    message: &str,
) -> Result<(), HeightmapDdsError> {
    let value =
        u32::try_from(value).map_err(|_| HeightmapDdsError::Message(message.to_string()))?;
    write_u32(writer, value)?;
    Ok(())
}

fn write_i32_checked(
    writer: &mut impl Write,
    value: u32,
    message: &str,
) -> Result<(), HeightmapDdsError> {
    let value =
        i32::try_from(value).map_err(|_| HeightmapDdsError::Message(message.to_string()))?;
    write_i32(writer, value)?;
    Ok(())
}

fn write_i32(writer: &mut impl Write, value: i32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u16(writer: &mut impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn checked_pixel_count(width: usize, height: usize) -> Result<usize, HeightmapDdsError> {
    checked_mul(width, height, "heightmap dimensions overflow")
}

fn checked_mul(lhs: usize, rhs: usize, message: &str) -> Result<usize, HeightmapDdsError> {
    lhs.checked_mul(rhs)
        .ok_or_else(|| HeightmapDdsError::Message(message.to_string()))
}
