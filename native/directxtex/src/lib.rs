mod blob;
mod constants;
mod dds_metadata;
mod dxgi_format;
mod enums;
mod ffi;
mod free_functions;
mod gpu;
mod hresult;
mod image;
mod macros;
pub mod output_directories;
pub mod profiling;
mod rect;
mod scratch_image;
mod texture_metadata;

pub use self::{
    blob::Blob,
    constants::{
        TEX_ALPHA_WEIGHT_DEFAULT, TEX_FILTER_DITHER_MASK, TEX_FILTER_MODE_MASK,
        TEX_FILTER_SRGB_MASK, TEX_THRESHOLD_DEFAULT,
    },
    dds_metadata::DDSMetaData,
    dxgi_format::{DXGI_FORMAT, Pitch, dxgi_format::*},
    enums::{
        CMSE_FLAGS, CNMAP_FLAGS, CP_FLAGS, DDS_FLAGS, FORMAT_TYPE, TEX_ALPHA_MODE,
        TEX_COMPRESS_FLAGS, TEX_DIMENSION, TEX_FILTER_FLAGS, TEX_MISC_FLAG, TEX_MISC_FLAG2,
        TEX_PMALPHA_FLAGS, TGA_FLAGS, cmse_flags::*, cnmap_flags::*, cp_flags::*, dds_flags::*,
        format_type::*, tex_alpha_mode::*, tex_compress_flags::*, tex_dimension::*,
        tex_filter_flags::*, tex_misc_flag::*, tex_misc_flag2::*, tex_pmalpha_flags::*,
        tga_flags::*,
    },
    free_functions::{
        compress, compute_normal_map, convert, convert_to_single_plane, decompress,
        generate_mip_maps, generate_mip_maps_3d, premultiply_alpha, resize, save_dds,
        scale_mip_maps_alpha_for_coverage,
    },
    hresult::HResultError,
    image::{Image, MeanSquaredError},
    rect::Rect,
    scratch_image::ScratchImage,
    texture_metadata::TexMetadata,
};
use std::fs;
use std::path::Path;
use std::slice;

mod ispc_bc;
mod python;

#[cfg(test)]
mod decode_bench;
#[cfg(test)]
mod encode_bench;

pub use gpu::{compress_bc7_gpu, compress_bc7_gpu_batch};
pub(crate) use hresult::HResult;
pub use python::register_module;

type Result<T> = core::result::Result<T, HResultError>;

#[derive(Debug, Clone)]
pub struct DdsRgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub dxgi_format: u32,
}

#[derive(Debug, Clone)]
pub struct DdsRgbaFloatImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    pub dxgi_format: u32,
}

fn parse_dxgi_format(name: &str) -> Option<DXGI_FORMAT> {
    match name.trim().to_ascii_uppercase().as_str() {
        "R8_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_R8_UNORM),
        "R8G8_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_R8G8_UNORM),
        "R8G8B8A8_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM),
        "R8G8B8A8_UNORM_SRGB" => Some(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM_SRGB),
        "B8G8R8A8_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM),
        "B8G8R8X8_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM),
        "BC1_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM),
        "BC1_UNORM_SRGB" => Some(DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM_SRGB),
        "BC2_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC2_UNORM),
        "BC2_UNORM_SRGB" => Some(DXGI_FORMAT::DXGI_FORMAT_BC2_UNORM_SRGB),
        "BC3_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM),
        "BC3_UNORM_SRGB" => Some(DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM_SRGB),
        "BC4_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC4_UNORM),
        "BC4_SNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC4_SNORM),
        "BC5_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM),
        "BC5_SNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC5_SNORM),
        "BC6H_UF16" => Some(DXGI_FORMAT::DXGI_FORMAT_BC6H_UF16),
        "BC6H_SF16" => Some(DXGI_FORMAT::DXGI_FORMAT_BC6H_SF16),
        "BC7_UNORM" => Some(DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM),
        "BC7_UNORM_SRGB" => Some(DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB),
        "R16G16B16A16_FLOAT" => Some(DXGI_FORMAT::DXGI_FORMAT_R16G16B16A16_FLOAT),
        _ => None,
    }
}

pub fn is_srgb_dxgi_format(dxgi_format: u32) -> bool {
    matches!(dxgi_format, 29 | 72 | 75 | 78 | 91 | 93 | 99)
}

fn legacy_dxt_target(name: &str) -> Option<(DXGI_FORMAT, &'static [u8; 4], usize)> {
    match name.trim().to_ascii_uppercase().as_str() {
        "DXT1" => Some((DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM, b"DXT1", 1)),
        "DXT5" => Some((DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM, b"DXT5", 2)),
        _ => None,
    }
}

fn compression_flags_for_format(
    format: DXGI_FORMAT,
    parallel_compression: bool,
) -> TEX_COMPRESS_FLAGS {
    let mut flags = if parallel_compression {
        TEX_COMPRESS_FLAGS::TEX_COMPRESS_PARALLEL
    } else {
        TEX_COMPRESS_FLAGS::TEX_COMPRESS_DEFAULT
    };
    if matches!(
        format,
        DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB
    ) {
        flags = flags.union(TEX_COMPRESS_FLAGS::TEX_COMPRESS_BC7_QUICK);
    }
    flags
}

fn packed_pixels_from_image(
    image: &Image,
    swizzle_bgra: bool,
    force_alpha_opaque: bool,
) -> core::result::Result<Vec<u8>, String> {
    let expected_row_pitch = image
        .width
        .checked_mul(4)
        .ok_or_else(|| "rgba row pitch overflow".to_string())?;
    let packed_len = expected_row_pitch
        .checked_mul(image.height)
        .ok_or_else(|| "rgba buffer size overflow".to_string())?;
    let src_bytes = unsafe { slice::from_raw_parts(image.pixels.cast_const(), image.slice_pitch) };

    if !swizzle_bgra && !force_alpha_opaque && image.row_pitch == expected_row_pitch {
        return Ok(src_bytes[..packed_len].to_vec());
    }

    let mut rgba = Vec::with_capacity(packed_len);
    for row in 0..image.height {
        let start = row
            .checked_mul(image.row_pitch)
            .ok_or_else(|| "rgba source offset overflow".to_string())?;
        let row_end = start
            .checked_add(expected_row_pitch)
            .ok_or_else(|| "rgba source slice overflow".to_string())?;
        let row_bytes = &src_bytes[start..row_end];
        if swizzle_bgra || force_alpha_opaque {
            for px in row_bytes.chunks_exact(4) {
                if swizzle_bgra {
                    rgba.push(px[2]);
                    rgba.push(px[1]);
                    rgba.push(px[0]);
                } else {
                    rgba.push(px[0]);
                    rgba.push(px[1]);
                    rgba.push(px[2]);
                }
                rgba.push(if force_alpha_opaque { 255 } else { px[3] });
            }
        } else {
            rgba.extend_from_slice(row_bytes);
        }
    }
    Ok(rgba)
}

fn read_u32_le(bytes: &[u8], offset: usize) -> core::result::Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "DDS header is truncated".to_string())?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn channel_from_mask(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let raw = (pixel & mask) >> shift;
    let max = mask >> shift;
    if max == 0 {
        255
    } else {
        ((raw * 255 + max / 2) / max) as u8
    }
}

fn try_load_legacy_rgba8_dds(
    bytes: &[u8],
) -> core::result::Result<Option<(u32, u32, Vec<u8>, u32)>, String> {
    if bytes.len() < 128 || bytes.get(0..4) != Some(b"DDS ") {
        return Ok(None);
    }
    let fourcc = bytes
        .get(84..88)
        .ok_or_else(|| "DDS header is truncated".to_string())?;
    let pf_flags = read_u32_le(bytes, 80)?;
    let rgb_bits = read_u32_le(bytes, 88)?;
    if fourcc == b"DX10" || (pf_flags & 0x40) == 0 || rgb_bits != 32 {
        return Ok(None);
    }

    let height = read_u32_le(bytes, 12)?;
    let width = read_u32_le(bytes, 16)?;
    let pitch = read_u32_le(bytes, 20)?.max(width.saturating_mul(4));
    let r_mask = read_u32_le(bytes, 92)?;
    let g_mask = read_u32_le(bytes, 96)?;
    let b_mask = read_u32_le(bytes, 100)?;
    let a_mask = if (pf_flags & 0x1) != 0 {
        read_u32_le(bytes, 104)?
    } else {
        0
    };
    let width_usize = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height_usize =
        usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    let pitch_usize = usize::try_from(pitch).map_err(|_| "pitch does not fit usize".to_string())?;
    let mut rgba = vec![
        0u8;
        width_usize
            .checked_mul(height_usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| "legacy DDS rgba buffer size overflow".to_string())?
    ];
    let data = bytes
        .get(128..)
        .ok_or_else(|| "legacy DDS pixel data is missing".to_string())?;
    for y in 0..height_usize {
        let row_start = y
            .checked_mul(pitch_usize)
            .ok_or_else(|| "legacy DDS row offset overflow".to_string())?;
        for x in 0..width_usize {
            let src_i = row_start
                .checked_add(x * 4)
                .ok_or_else(|| "legacy DDS pixel offset overflow".to_string())?;
            let pixel_bytes = data
                .get(src_i..src_i + 4)
                .ok_or_else(|| "legacy DDS pixel data is truncated".to_string())?;
            let pixel = u32::from_le_bytes(pixel_bytes.try_into().unwrap());
            let dst_i = (y * width_usize + x) * 4;
            rgba[dst_i] = channel_from_mask(pixel, r_mask);
            rgba[dst_i + 1] = channel_from_mask(pixel, g_mask);
            rgba[dst_i + 2] = channel_from_mask(pixel, b_mask);
            rgba[dst_i + 3] = if a_mask == 0 {
                255
            } else {
                channel_from_mask(pixel, a_mask)
            };
        }
    }

    Ok(Some((
        width,
        height,
        rgba,
        DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM.bits(),
    )))
}

#[derive(Debug)]
pub(crate) struct TexdiagInfo {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub format_bits: u32,
    pub format_name: String,
    pub dimension: String,
    pub alpha_mode: String,
    pub is_cubemap: bool,
    pub is_compressed: bool,
    pub has_alpha: bool,
    pub is_dx10: bool,
    pub is_xbox: bool,
    pub is_power_of_two: bool,
    pub bits_per_pixel: usize,
    pub bits_per_color: usize,
    pub image_count: usize,
    pub file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsValidationFinding {
    pub severity: &'static str,
    pub rule: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsValidationReport {
    pub width: u32,
    pub height: u32,
    pub findings: Vec<DdsValidationFinding>,
}

pub fn validate_dds_bytes(
    bytes: &[u8],
    include_optional: bool,
) -> core::result::Result<DdsValidationReport, String> {
    if bytes.len() < 128 || bytes.get(0..4) != Some(b"DDS ") {
        return Err("Not a valid DDS file".to_string());
    }
    if read_u32_le(bytes, 4)? != 124 || read_u32_le(bytes, 76)? != 32 {
        return Err("Not a valid DDS file".to_string());
    }

    let height = read_u32_le(bytes, 12)?;
    let width = read_u32_le(bytes, 16)?;
    let pixel_flags = read_u32_le(bytes, 80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or_else(|| "DDS header is truncated".to_string())?;
    let rgb_bits = read_u32_le(bytes, 88)?;
    let red_mask = read_u32_le(bytes, 92)?;
    let green_mask = read_u32_le(bytes, 96)?;
    let blue_mask = read_u32_le(bytes, 100)?;
    let alpha_mask = read_u32_le(bytes, 104)?;
    let mut findings = Vec::new();

    if !width.is_power_of_two() || !height.is_power_of_two() {
        findings.push(DdsValidationFinding {
            severity: "error",
            rule: "invalid-texture-size-format",
            message: format!("Texture size {width}x{height} is not power of 2"),
        });
    }

    if legacy_format_without_dxgi(
        pixel_flags,
        fourcc,
        rgb_bits,
        red_mask,
        green_mask,
        blue_mask,
        alpha_mask,
        bytes,
    ) == Some("R8G8B8")
    {
        findings.push(DdsValidationFinding {
            severity: "error",
            rule: "invalid-texture-size-format",
            message: "R8G8B8 format is unsupported by DirectX 10+ (Skyrim SE, Fallout 4, etc.)"
                .to_string(),
        });
    }

    if include_optional
        && pixel_flags & 0x40 != 0
        && (red_mask != 0x00FF_0000 || green_mask != 0x0000_FF00 || blue_mask != 0x0000_00FF)
    {
        findings.push(DdsValidationFinding {
            severity: "error",
            rule: "sse-unsupported-texture-format",
            message: "Texture format is not supported by Skyrim SE on Windows 7".to_string(),
        });
    }

    Ok(DdsValidationReport {
        width,
        height,
        findings,
    })
}

pub fn validate_dds_file(
    path: &Path,
    include_optional: bool,
) -> core::result::Result<DdsValidationReport, String> {
    let bytes = crate::profiling::read(path).map_err(|error| error.to_string())?;
    validate_dds_bytes(&bytes, include_optional)
}

fn legacy_format_without_dxgi(
    pixel_flags: u32,
    fourcc: &[u8],
    rgb_bits: u32,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
    bytes: &[u8],
) -> Option<&'static str> {
    let known_fourcc = matches!(
        fourcc,
        b"DXT1" | b"DXT3" | b"DXT5" | b"ATI1" | b"ATI2" | b"BC4S" | b"BC4U" | b"BC5S" | b"BC5U"
    );
    if known_fourcc {
        return None;
    }
    if matches!(fourcc, b"DX10" | b"XBOX") {
        return if bytes.len() >= 148
            && read_u32_le(bytes, 128)
                .ok()
                .is_some_and(|format| format != 0)
        {
            None
        } else {
            Some("UNKNOWN")
        };
    }
    if pixel_flags & (0x40 | 0x20_000) == 0 {
        return None;
    }

    match rgb_bits {
        32 | 16 | 8 => None,
        24 if red_mask == 0x00FF_0000
            && green_mask == 0x0000_FF00
            && blue_mask == 0x0000_00FF
            && alpha_mask == 0 =>
        {
            Some("R8G8B8")
        }
        _ => Some("UNKNOWN"),
    }
}

fn dxgi_format_name(format: DXGI_FORMAT) -> String {
    let name = format!("{format:?}");
    name.strip_prefix("DXGI_FORMAT_")
        .unwrap_or(&name)
        .to_string()
}

fn alpha_mode_name(mode: TEX_ALPHA_MODE) -> &'static str {
    match mode {
        TEX_ALPHA_MODE::TEX_ALPHA_MODE_STRAIGHT => "straight",
        TEX_ALPHA_MODE::TEX_ALPHA_MODE_PREMULTIPLIED => "premultiplied",
        TEX_ALPHA_MODE::TEX_ALPHA_MODE_OPAQUE => "opaque",
        TEX_ALPHA_MODE::TEX_ALPHA_MODE_CUSTOM => "custom",
        _ => "unknown",
    }
}

fn dimension_name(metadata: &TexMetadata) -> &'static str {
    match metadata.dimension {
        TEX_DIMENSION::TEX_DIMENSION_TEXTURE1D => {
            if metadata.array_size > 1 {
                "1DArray"
            } else {
                "1D"
            }
        }
        TEX_DIMENSION::TEX_DIMENSION_TEXTURE2D => {
            if metadata.is_cubemap() {
                if metadata.array_size > 6 {
                    "CubeArray"
                } else {
                    "Cube"
                }
            } else if metadata.array_size > 1 {
                "2DArray"
            } else {
                "2D"
            }
        }
        TEX_DIMENSION::TEX_DIMENSION_TEXTURE3D => "3D",
        _ => "unknown",
    }
}

fn try_legacy_rgba8_info(
    bytes: &[u8],
    file_size: u64,
) -> core::result::Result<Option<TexdiagInfo>, String> {
    if bytes.len() < 128 || bytes.get(0..4) != Some(b"DDS ") {
        return Ok(None);
    }
    let fourcc = bytes
        .get(84..88)
        .ok_or_else(|| "DDS header is truncated".to_string())?;
    let pf_flags = read_u32_le(bytes, 80)?;
    let rgb_bits = read_u32_le(bytes, 88)?;
    if fourcc == b"DX10" || (pf_flags & 0x40) == 0 || rgb_bits != 32 {
        return Ok(None);
    }
    let mip_levels = read_u32_le(bytes, 28)?.max(1);
    let format = DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM;
    Ok(Some(TexdiagInfo {
        width: read_u32_le(bytes, 16)?,
        height: read_u32_le(bytes, 12)?,
        depth: read_u32_le(bytes, 24)?.max(1),
        mip_levels,
        array_size: 1,
        format_bits: format.bits(),
        format_name: dxgi_format_name(format),
        dimension: "2D".to_string(),
        alpha_mode: "unknown".to_string(),
        is_cubemap: false,
        is_compressed: false,
        has_alpha: read_u32_le(bytes, 104)? != 0,
        is_dx10: false,
        is_xbox: false,
        is_power_of_two: read_u32_le(bytes, 16)?.is_power_of_two()
            && read_u32_le(bytes, 12)?.is_power_of_two(),
        bits_per_pixel: format.bits_per_pixel(),
        bits_per_color: format.bits_per_color(),
        image_count: usize::try_from(mip_levels)
            .map_err(|_| "mip count does not fit usize".to_string())?,
        file_size,
    }))
}

pub(crate) fn texdiag_info_bytes(path: &Path) -> core::result::Result<TexdiagInfo, String> {
    let bytes = crate::profiling::read(path).map_err(|err| err.to_string())?;
    let file_size = u64::try_from(bytes.len()).map_err(|_| "file size overflow".to_string())?;
    if let Some(info) = try_legacy_rgba8_info(&bytes, file_size)? {
        return Ok(info);
    }

    let scratch = ScratchImage::load_dds(&bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
        .map_err(|err| err.to_string())?;
    let metadata = *scratch.metadata();
    let width = metadata
        .width
        .try_into()
        .map_err(|_| "dds width exceeds u32".to_string())?;
    let height = metadata
        .height
        .try_into()
        .map_err(|_| "dds height exceeds u32".to_string())?;
    let depth = metadata
        .depth
        .try_into()
        .map_err(|_| "dds depth exceeds u32".to_string())?;
    let mip_levels = metadata
        .mip_levels
        .try_into()
        .map_err(|_| "dds mip count exceeds u32".to_string())?;
    let array_size = metadata
        .array_size
        .try_into()
        .map_err(|_| "dds array size exceeds u32".to_string())?;
    Ok(TexdiagInfo {
        width,
        height,
        depth,
        mip_levels,
        array_size,
        format_bits: metadata.format.bits(),
        format_name: dxgi_format_name(metadata.format),
        dimension: dimension_name(&metadata).to_string(),
        alpha_mode: alpha_mode_name(metadata.get_alpha_mode()).to_string(),
        is_cubemap: metadata.is_cubemap(),
        is_compressed: metadata.format.is_compressed(),
        has_alpha: metadata.format.has_alpha(),
        is_dx10: bytes.get(84..88) == Some(b"DX10"),
        is_xbox: bytes.get(84..88) == Some(b"XBOX"),
        is_power_of_two: width.is_power_of_two() && height.is_power_of_two(),
        bits_per_pixel: metadata.format.bits_per_pixel(),
        bits_per_color: metadata.format.bits_per_color(),
        image_count: scratch.images().len(),
        file_size,
    })
}

fn decode_bc4_unorm_block(block: &[u8]) -> [u8; 16] {
    let e0 = block[0];
    let e1 = block[1];
    let mut palette = [0u8; 8];
    palette[0] = e0;
    palette[1] = e1;
    if e0 > e1 {
        for i in 1..=6 {
            let v = ((7 - i) as u16 * u16::from(e0) + i as u16 * u16::from(e1) + 3) / 7;
            palette[i + 1] = v as u8;
        }
    } else {
        for i in 1..=4 {
            let v = ((5 - i) as u16 * u16::from(e0) + i as u16 * u16::from(e1) + 2) / 5;
            palette[i + 1] = v as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }

    let mut bits = 0u64;
    for i in 0..6 {
        bits |= u64::from(block[2 + i]) << (8 * i);
    }

    let mut out = [0u8; 16];
    for (i, dst) in out.iter_mut().enumerate() {
        let idx = ((bits >> (3 * i)) & 0x7) as usize;
        *dst = palette[idx];
    }
    out
}

fn decode_bc5_unorm_image(image: &Image) -> core::result::Result<Vec<u8>, String> {
    let width = image.width;
    let height = image.height;
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let src_bytes = unsafe { slice::from_raw_parts(image.pixels.cast_const(), image.slice_pitch) };
    let mut rgba = vec![
        0u8;
        width
            .checked_mul(height)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| "bc5 rgba buffer size overflow".to_string())?
    ];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let offset = by
                .checked_mul(image.row_pitch)
                .and_then(|row| row.checked_add(bx * 16))
                .ok_or_else(|| "bc5 source offset overflow".to_string())?;
            let block = src_bytes
                .get(offset..offset + 16)
                .ok_or_else(|| "bc5 block exceeds source buffer".to_string())?;
            let red = decode_bc4_unorm_block(&block[0..8]);
            let green = decode_bc4_unorm_block(&block[8..16]);

            for y in 0..4 {
                let py = by * 4 + y;
                if py >= height {
                    continue;
                }
                for x in 0..4 {
                    let px = bx * 4 + x;
                    if px >= width {
                        continue;
                    }
                    let src_i = y * 4 + x;
                    let dst_i = (py * width + px) * 4;
                    rgba[dst_i] = red[src_i];
                    rgba[dst_i + 1] = green[src_i];
                    rgba[dst_i + 2] = 0;
                    rgba[dst_i + 3] = 255;
                }
            }
        }
    }
    Ok(rgba)
}

fn encode_bc4_unorm_block(values: &[u8; 16]) -> [u8; 8] {
    let min = *values.iter().min().unwrap_or(&0);
    let max = *values.iter().max().unwrap_or(&0);
    let mut out = [0u8; 8];
    out[0] = max;
    out[1] = min;

    let mut palette = [0u8; 8];
    palette[0] = max;
    palette[1] = min;
    if max == min {
        palette.fill(max);
    } else {
        for i in 1..=6 {
            let v = ((7 - i) as u16 * u16::from(max) + i as u16 * u16::from(min) + 3) / 7;
            palette[i + 1] = v as u8;
        }
    }

    let mut bits = 0u64;
    for (i, value) in values.iter().enumerate() {
        let mut best_idx = 0usize;
        let mut best_err = u16::MAX;
        for (idx, candidate) in palette.iter().enumerate() {
            let err = u16::from(value.abs_diff(*candidate));
            if err < best_err {
                best_idx = idx;
                best_err = err;
            }
        }
        bits |= (best_idx as u64) << (3 * i);
    }

    for i in 0..6 {
        out[2 + i] = ((bits >> (8 * i)) & 0xff) as u8;
    }
    out
}

fn push_u32_le(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn dds_header(
    width: u32,
    height: u32,
    linear_size: u32,
    fourcc: &[u8; 4],
    mip_count: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    let has_mips = mip_count > 1;
    out.extend_from_slice(b"DDS ");
    push_u32_le(&mut out, 124);
    push_u32_le(&mut out, if has_mips { 0x000a_1007 } else { 0x0008_1007 });
    push_u32_le(&mut out, height);
    push_u32_le(&mut out, width);
    push_u32_le(&mut out, linear_size);
    push_u32_le(&mut out, 0);
    push_u32_le(&mut out, if has_mips { mip_count } else { 0 });
    for _ in 0..11 {
        push_u32_le(&mut out, 0);
    }
    push_u32_le(&mut out, 32);
    push_u32_le(&mut out, 0x0000_0004);
    out.extend_from_slice(fourcc);
    for _ in 0..5 {
        push_u32_le(&mut out, 0);
    }
    push_u32_le(&mut out, if has_mips { 0x0040_1008 } else { 0x0000_1000 });
    for _ in 0..4 {
        push_u32_le(&mut out, 0);
    }
    out
}

fn dds_rgba8_header(width: u32, height: u32, pitch: u32, mip_count: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    let has_mips = mip_count > 1;
    out.extend_from_slice(b"DDS ");
    push_u32_le(&mut out, 124);
    push_u32_le(&mut out, if has_mips { 0x0002_100f } else { 0x0000_100f });
    push_u32_le(&mut out, height);
    push_u32_le(&mut out, width);
    push_u32_le(&mut out, pitch);
    push_u32_le(&mut out, 0);
    push_u32_le(&mut out, if has_mips { mip_count } else { 0 });
    for _ in 0..11 {
        push_u32_le(&mut out, 0);
    }
    push_u32_le(&mut out, 32);
    push_u32_le(&mut out, 0x0000_0041);
    push_u32_le(&mut out, 0);
    push_u32_le(&mut out, 32);
    push_u32_le(&mut out, 0x0000_00ff);
    push_u32_le(&mut out, 0x0000_ff00);
    push_u32_le(&mut out, 0x00ff_0000);
    push_u32_le(&mut out, 0xff00_0000);
    push_u32_le(&mut out, if has_mips { 0x0040_1008 } else { 0x0000_1000 });
    for _ in 0..4 {
        push_u32_le(&mut out, 0);
    }
    out
}

fn dds_dx10_header(
    width: u32,
    height: u32,
    linear_size: u32,
    format: DXGI_FORMAT,
    mip_count: u32,
) -> Vec<u8> {
    let mut out = dds_header(width, height, linear_size, b"DX10", mip_count);
    push_u32_le(&mut out, format.bits());
    push_u32_le(&mut out, 3);
    push_u32_le(&mut out, 0);
    push_u32_le(&mut out, 1);
    push_u32_le(&mut out, 0);
    out
}

fn bc_level_size(
    width: usize,
    height: usize,
    channels: usize,
) -> core::result::Result<usize, String> {
    width
        .div_ceil(4)
        .checked_mul(height.div_ceil(4))
        .and_then(|blocks| blocks.checked_mul(if channels == 1 { 8 } else { 16 }))
        .ok_or_else(|| "bc output size overflow".to_string())
}

fn downsample_rgba_2x(
    src: &[u8],
    width: usize,
    height: usize,
) -> core::result::Result<(usize, usize, Vec<u8>), String> {
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut dst = vec![
        0u8;
        next_width
            .checked_mul(next_height)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| "mip rgba buffer size overflow".to_string())?
    ];

    for y in 0..next_height {
        for x in 0..next_width {
            let mut sum = [0u16; 4];
            let mut count = 0u16;
            for oy in 0..2 {
                let sy = y * 2 + oy;
                if sy >= height {
                    continue;
                }
                for ox in 0..2 {
                    let sx = x * 2 + ox;
                    if sx >= width {
                        continue;
                    }
                    let src_i = (sy * width + sx) * 4;
                    for c in 0..4 {
                        sum[c] += u16::from(src[src_i + c]);
                    }
                    count += 1;
                }
            }
            let dst_i = (y * next_width + x) * 4;
            for c in 0..4 {
                dst[dst_i + c] = ((sum[c] + count / 2) / count) as u8;
            }
        }
    }

    Ok((next_width, next_height, dst))
}

fn rgba_mip_chain(
    width: usize,
    height: usize,
    rgba: &[u8],
    generate_mips: bool,
) -> core::result::Result<Vec<(usize, usize, Vec<u8>)>, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Mips);
    let mut chain = vec![(width, height, rgba.to_vec())];
    if !generate_mips {
        return Ok(chain);
    }

    while let Some((last_width, last_height, last_rgba)) = chain.last() {
        if *last_width == 1 && *last_height == 1 {
            break;
        }
        let next = downsample_rgba_2x(last_rgba, *last_width, *last_height)?;
        chain.push(next);
    }

    Ok(chain)
}

/// Independent RGBA8 implementation of the mip-flood method published in
/// Santa Monica Studio's 2019 GDC presentation. The alpha-weighted pyramid is
/// used to flood the full-resolution RGB buffer, then ordinary box-filtered
/// DDS mips are generated from that flooded base. Alpha is never modified.
pub fn rgba8_mip_flood_chain(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> core::result::Result<Vec<(u32, u32, Vec<u8>)>, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Mips);
    let width = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height = usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    if width == 0 || height == 0 {
        return Err("mip flood dimensions must be non-zero".to_string());
    }
    let expected_len = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "mip flood rgba buffer size overflow".to_string())?;
    if rgba.len() != expected_len {
        return Err(format!(
            "mip flood rgba buffer length mismatch: expected {expected_len}, got {}",
            rgba.len()
        ));
    }

    let mut chain: Vec<(u32, u32, Vec<u8>)> = rgba_mip_chain(width, height, rgba, true)?
        .into_iter()
        .map(|(w, h, pixels)| (w as u32, h as u32, pixels))
        .collect();
    if !rgba.chunks_exact(4).any(|pixel| pixel[3] != 0) {
        return Ok(chain);
    }

    let mut previous_weights: Option<Vec<f32>> = None;
    for level in 1..chain.len() {
        let (previous_levels, next_levels) = chain.split_at_mut(level);
        let (previous_width, previous_height, previous_pixels) = &previous_levels[level - 1];
        let (next_width, next_height, next_pixels) = &mut next_levels[0];
        let mut next_weights = vec![0.0f32; (*next_width * *next_height) as usize];

        for y in 0..*next_height {
            for x in 0..*next_width {
                let mut weighted_rgb = [0.0f32; 3];
                let mut weight_sum = 0.0f32;
                let mut count = 0.0f32;
                for oy in 0..2 {
                    let source_y = y * 2 + oy;
                    if source_y >= *previous_height {
                        continue;
                    }
                    for ox in 0..2 {
                        let source_x = x * 2 + ox;
                        if source_x >= *previous_width {
                            continue;
                        }
                        let source_pixel = (source_y * *previous_width + source_x) as usize;
                        let source_offset = source_pixel * 4;
                        let weight = previous_weights
                            .as_ref()
                            .map(|weights| weights[source_pixel])
                            .unwrap_or_else(|| {
                                f32::from(previous_pixels[source_offset + 3]) / 255.0
                            });
                        for channel in 0..3 {
                            weighted_rgb[channel] +=
                                f32::from(previous_pixels[source_offset + channel]) * weight;
                        }
                        weight_sum += weight;
                        count += 1.0;
                    }
                }

                let next_pixel = (y * *next_width + x) as usize;
                let next_offset = next_pixel * 4;
                if weight_sum > 0.0 {
                    for channel in 0..3 {
                        next_pixels[next_offset + channel] =
                            (weighted_rgb[channel] / weight_sum).round() as u8;
                    }
                }
                next_weights[next_pixel] = weight_sum / count;
            }
        }
        previous_weights = Some(next_weights);
    }

    for level in (0..chain.len().saturating_sub(1)).rev() {
        let (higher_levels, lower_levels) = chain.split_at_mut(level + 1);
        let (width, height, pixels) = &mut higher_levels[level];
        let (lower_width, lower_height, lower_pixels) = &lower_levels[0];
        for y in 0..*height {
            for x in 0..*width {
                let pixel_i = ((y * *width + x) * 4) as usize;
                let lower_x = (x * *lower_width / *width).min(*lower_width - 1);
                let lower_y = (y * *lower_height / *height).min(*lower_height - 1);
                let lower_i = ((lower_y * *lower_width + lower_x) * 4) as usize;
                let alpha = u32::from(pixels[pixel_i + 3]);
                for channel in 0..3 {
                    let foreground = u32::from(pixels[pixel_i + channel]);
                    let background = u32::from(lower_pixels[lower_i + channel]);
                    pixels[pixel_i + channel] =
                        ((foreground * alpha + background * (255 - alpha) + 127) / 255) as u8;
                }
            }
        }
    }

    rgba8_box_mip_chain(width as u32, height as u32, &chain[0].2)
}

pub fn mip_flood_output_format<'a>(format: &'a str, rgba: &[u8]) -> &'a str {
    let has_color_seed = rgba.chunks_exact(4).any(|pixel| pixel[3] != 0);
    let has_transparency = rgba.chunks_exact(4).any(|pixel| pixel[3] != 255);
    if !has_color_seed || !has_transparency {
        return format;
    }
    // BC1's transparent selector decodes RGB as black, which destroys the
    // hidden colors flooding exists to create. BC3 stores color and alpha separately.
    if format.eq_ignore_ascii_case("BC1_UNORM_SRGB") {
        "BC3_UNORM_SRGB"
    } else if format.eq_ignore_ascii_case("BC1_UNORM") {
        "BC3_UNORM"
    } else {
        format
    }
}

fn append_bc_unorm_level(
    out: &mut Vec<u8>,
    width: usize,
    height: usize,
    rgba: &[u8],
    channels: usize,
) -> core::result::Result<(), String> {
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let mut red = [0u8; 16];
            let mut green = [0u8; 16];
            for y in 0..4 {
                let py = (by * 4 + y).min(height - 1);
                for x in 0..4 {
                    let px = (bx * 4 + x).min(width - 1);
                    let src_i = (py * width + px) * 4;
                    let dst_i = y * 4 + x;
                    red[dst_i] = rgba[src_i];
                    green[dst_i] = rgba[src_i + 1];
                }
            }
            out.extend_from_slice(&encode_bc4_unorm_block(&red));
            if channels == 2 {
                out.extend_from_slice(&encode_bc4_unorm_block(&green));
            }
        }
    }

    Ok(())
}

fn encode_bc_unorm_dds(
    width: u32,
    height: u32,
    rgba: &[u8],
    channels: usize,
    generate_mips: bool,
) -> core::result::Result<Vec<u8>, String> {
    let width_usize = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height_usize =
        usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    let chain = rgba_mip_chain(width_usize, height_usize, rgba, generate_mips)?;
    let linear_size = bc_level_size(width_usize, height_usize, channels)?;
    let fourcc = if channels == 1 { b"ATI1" } else { b"ATI2" };
    let mip_count =
        u32::try_from(chain.len()).map_err(|_| "mip count does not fit u32".to_string())?;
    let mut out = dds_header(width, height, linear_size as u32, fourcc, mip_count);

    for (level_width, level_height, level_rgba) in chain {
        if channels == 2 {
            out.extend_from_slice(&ispc_bc::bc5_blocks_from_rgba(
                level_width,
                level_height,
                &level_rgba,
            )?);
        } else {
            append_bc_unorm_level(&mut out, level_width, level_height, &level_rgba, channels)?;
        }
    }
    Ok(out)
}

fn encode_rgba8_dds(
    width: u32,
    height: u32,
    rgba: &[u8],
    format: DXGI_FORMAT,
    generate_mips: bool,
) -> core::result::Result<Vec<u8>, String> {
    let width_usize = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height_usize =
        usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    let chain = rgba_mip_chain(width_usize, height_usize, rgba, generate_mips)?;
    let mip_count =
        u32::try_from(chain.len()).map_err(|_| "mip count does not fit u32".to_string())?;
    let pitch = width
        .checked_mul(4)
        .ok_or_else(|| "dds pitch overflow".to_string())?;
    let mut out = if format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        dds_rgba8_header(width, height, pitch, mip_count)
    } else {
        dds_dx10_header(width, height, pitch, format, mip_count)
    };
    for (_, _, level_rgba) in chain {
        out.extend_from_slice(&level_rgba);
    }
    Ok(out)
}

fn scratch_for_target_compression(
    scratch: ScratchImage,
    target_format: DXGI_FORMAT,
) -> core::result::Result<ScratchImage, String> {
    if target_format.is_srgb() {
        scratch
            .convert(
                DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
                TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                TEX_THRESHOLD_DEFAULT,
            )
            .map_err(|err| err.to_string())
    } else {
        Ok(scratch)
    }
}

fn compressed_payload_from_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
    target_format: DXGI_FORMAT,
    parallel_compression: bool,
    use_gpu: bool,
) -> core::result::Result<Vec<u8>, String> {
    if use_gpu
        && matches!(
            target_format,
            DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB
        )
    {
        let srgb = matches!(target_format, DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB);
        if let (Ok(w), Ok(h)) = (u32::try_from(width), u32::try_from(height)) {
            if let Ok(bytes) = crate::gpu::compress_bc7_gpu(rgba, w, h, srgb) {
                return Ok(bytes);
            }
        }
        // fall through to CPU on any GPU error
    }
    match target_format {
        DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM_SRGB
            if rgba.chunks_exact(4).all(|pixel| pixel[3] == 0xFF) =>
        {
            return ispc_bc1_payload(width, height, rgba, target_format.is_srgb());
        }
        DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM_SRGB => {
            return ispc_bc3_payload(width, height, rgba, target_format.is_srgb());
        }
        DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB => {
            return ispc_bc7_payload(width, height, rgba, target_format.is_srgb());
        }
        _ => {}
    }
    dxtex_compressed_payload(width, height, rgba, target_format, parallel_compression)
}

fn ispc_bc1_payload(
    width: usize,
    height: usize,
    rgba: &[u8],
    srgb: bool,
) -> core::result::Result<Vec<u8>, String> {
    if srgb {
        let converted = convert_unorm_texels_to_srgb(width, height, rgba)?;
        ispc_bc::bc1_blocks_from_rgba(width, height, &converted)
    } else {
        ispc_bc::bc1_blocks_from_rgba(width, height, rgba)
    }
}

fn ispc_bc3_payload(
    width: usize,
    height: usize,
    rgba: &[u8],
    srgb: bool,
) -> core::result::Result<Vec<u8>, String> {
    if srgb {
        let converted = convert_unorm_texels_to_srgb(width, height, rgba)?;
        ispc_bc::bc3_blocks_from_rgba(width, height, &converted)
    } else {
        ispc_bc::bc3_blocks_from_rgba(width, height, rgba)
    }
}

/// BC7 CPU encode via the ISPC kernel. sRGB targets keep the exact pre-encode
/// texel conversion the DirectXTex path performed (UNORM -> UNORM_SRGB via
/// DirectXTex Convert); the kernel then packs blocks from the same texels the
/// old encoder saw.
fn ispc_bc7_payload(
    width: usize,
    height: usize,
    rgba: &[u8],
    srgb: bool,
) -> core::result::Result<Vec<u8>, String> {
    if srgb {
        let converted = convert_unorm_texels_to_srgb(width, height, rgba)?;
        let settings = ispc_bc::bc7_production_settings(&converted);
        ispc_bc::bc7_blocks_from_rgba(width, height, &converted, &settings)
    } else {
        let settings = ispc_bc::bc7_production_settings(rgba);
        ispc_bc::bc7_blocks_from_rgba(width, height, rgba, &settings)
    }
}

fn convert_unorm_texels_to_srgb(
    width: usize,
    height: usize,
    rgba: &[u8],
) -> core::result::Result<Vec<u8>, String> {
    let mut scratch = ScratchImage::default();
    scratch
        .initialize_2d(
            DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
            width,
            height,
            1,
            1,
            CP_FLAGS::CP_FLAGS_NONE,
        )
        .map_err(|err| err.to_string())?;
    scratch.pixels_mut().copy_from_slice(rgba);
    let converted =
        scratch_for_target_compression(scratch, DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB)?;
    let image = converted
        .image(0, 0, 0)
        .ok_or_else(|| "converted image missing".to_string())?;
    packed_pixels_from_image(image, false, false)
}

pub fn convert_srgb_texels_to_linear_unorm(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> core::result::Result<Vec<u8>, String> {
    let width = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height = usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "rgba buffer size overflow".to_string())?;
    if rgba.len() != expected_len {
        return Err(format!(
            "rgba buffer length mismatch: expected {expected_len} bytes, got {}",
            rgba.len()
        ));
    }

    let mut scratch = ScratchImage::default();
    scratch
        .initialize_2d(
            DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            width,
            height,
            1,
            1,
            CP_FLAGS::CP_FLAGS_NONE,
        )
        .map_err(|err| err.to_string())?;
    scratch.pixels_mut().copy_from_slice(rgba);
    let converted = scratch
        .convert(
            DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
            TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
            TEX_THRESHOLD_DEFAULT,
        )
        .map_err(|err| err.to_string())?;
    let image = converted
        .image(0, 0, 0)
        .ok_or_else(|| "converted image missing".to_string())?;
    packed_pixels_from_image(image, false, false)
}

fn dxtex_compressed_payload(
    width: usize,
    height: usize,
    rgba: &[u8],
    target_format: DXGI_FORMAT,
    parallel_compression: bool,
) -> core::result::Result<Vec<u8>, String> {
    let mut scratch = ScratchImage::default();
    scratch
        .initialize_2d(
            DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
            width,
            height,
            1,
            1,
            CP_FLAGS::CP_FLAGS_NONE,
        )
        .map_err(|err| err.to_string())?;
    scratch.pixels_mut().copy_from_slice(rgba);
    let scratch = scratch_for_target_compression(scratch, target_format)?;

    let encoded = scratch
        .compress(
            target_format,
            compression_flags_for_format(target_format, parallel_compression),
            TEX_THRESHOLD_DEFAULT,
        )
        .and_then(|compressed| compressed.save_dds(DDS_FLAGS::DDS_FLAGS_NONE))
        .map_err(|err| err.to_string())?;
    let bytes = encoded.buffer();
    let payload_offset = if bytes.get(84..88) == Some(b"DX10") {
        148
    } else {
        128
    };
    bytes
        .get(payload_offset..)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "compressed DDS output missing payload".to_string())
}

fn encode_compressed_dds(
    width: u32,
    height: u32,
    rgba: &[u8],
    target_format: DXGI_FORMAT,
    generate_mips: bool,
    parallel_compression: bool,
    use_gpu: bool,
) -> core::result::Result<Vec<u8>, String> {
    let width_usize = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height_usize =
        usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    let chain = rgba_mip_chain(width_usize, height_usize, rgba, generate_mips)?;
    let block_channels = if matches!(
        target_format,
        DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM_SRGB
    ) {
        1
    } else {
        2
    };
    let linear_size = bc_level_size(width_usize, height_usize, block_channels)?;
    let mip_count =
        u32::try_from(chain.len()).map_err(|_| "mip count does not fit u32".to_string())?;
    let mut out = dds_dx10_header(width, height, linear_size as u32, target_format, mip_count);

    // Fast path: encode the whole mip chain in one GPU submission that reuses a
    // cached BC7 compute pipeline. On any GPU error we fall through to the
    // per-mip loop below, which is byte-identical to the pre-batch behavior.
    if use_gpu
        && matches!(
            target_format,
            DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB
        )
    {
        let srgb = matches!(target_format, DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB);
        let images: Vec<(u32, u32, &[u8])> = chain
            .iter()
            .filter_map(
                |(w, h, rgba)| match (u32::try_from(*w), u32::try_from(*h)) {
                    (Ok(w), Ok(h)) => Some((w, h, rgba.as_slice())),
                    _ => None,
                },
            )
            .collect();
        if images.len() == chain.len() {
            if let Ok(payloads) = crate::gpu::compress_bc7_gpu_batch(&images, srgb) {
                for payload in payloads {
                    out.extend_from_slice(&payload);
                }
                return Ok(out);
            }
        }
    }

    for (level_width, level_height, level_rgba) in chain {
        out.extend_from_slice(&compressed_payload_from_rgba(
            level_width,
            level_height,
            &level_rgba,
            target_format,
            parallel_compression,
            use_gpu,
        )?);
    }
    Ok(out)
}

fn encode_legacy_dxt_dds_from_chain(
    chain: &[(u32, u32, Vec<u8>)],
    target_format: DXGI_FORMAT,
    fourcc: &[u8; 4],
    block_channels: usize,
    parallel_compression: bool,
) -> core::result::Result<Vec<u8>, String> {
    let (width, height) = chain
        .first()
        .map(|(w, h, _)| (*w, *h))
        .ok_or_else(|| "empty mip chain".to_string())?;
    let linear_size = bc_level_size(width as usize, height as usize, block_channels)?;
    let mip_count =
        u32::try_from(chain.len()).map_err(|_| "mip count does not fit u32".to_string())?;
    let mut out = dds_header(width, height, linear_size as u32, fourcc, mip_count);

    for (level_width, level_height, level_rgba) in chain {
        out.extend_from_slice(&compressed_payload_from_rgba(
            *level_width as usize,
            *level_height as usize,
            level_rgba,
            target_format,
            parallel_compression,
            false,
        )?);
    }
    Ok(out)
}

pub(crate) fn dds_base_rgba(path: &Path) -> core::result::Result<(u32, u32, Vec<u8>, u32), String> {
    let bytes = crate::profiling::read(path).map_err(|err| err.to_string())?;
    dds_base_rgba_bytes(&bytes)
}

fn dds_base_rgba_bytes(bytes: &[u8]) -> core::result::Result<(u32, u32, Vec<u8>, u32), String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Decode);
    if let Some(decoded) = try_load_legacy_rgba8_dds(bytes)? {
        return Ok(decoded);
    }
    let scratch = ScratchImage::load_dds(bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
        .map_err(|err| err.to_string())?;
    let metadata = *scratch.metadata();
    let width: u32 = metadata
        .width
        .try_into()
        .map_err(|_| "dds width exceeds u32".to_string())?;
    let height: u32 = metadata
        .height
        .try_into()
        .map_err(|_| "dds height exceeds u32".to_string())?;
    let dxgi_format = metadata.format.bits();

    // FO4 envmaps are legacy BGRA cubemaps. DirectXTex's generic convert/decompress
    // path can fail on these with E_NOINTERFACE, but we only need the first face's
    // top mip as a 2D texture in the editor. Read it directly and swizzle to RGBA.
    if metadata.is_cubemap()
        && matches!(
            metadata.format,
            DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
        )
    {
        let face0 = scratch
            .image(0, 0, 0)
            .ok_or_else(|| "dds cubemap does not contain a readable base face".to_string())?;
        let rgba = packed_pixels_from_image(
            face0,
            true,
            matches!(
                metadata.format,
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
                    | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
            ),
        )?;
        return Ok((width, height, rgba, dxgi_format));
    }

    if metadata.format == DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM {
        let image = scratch
            .image(0, 0, 0)
            .ok_or_else(|| "dds BC5 image does not contain a readable base mip".to_string())?;
        let rgba = decode_bc5_unorm_image(image)?;
        return Ok((width, height, rgba, dxgi_format));
    }

    // Plain 2D callers only consume mip 0, so avoid converting/decompressing the
    // discarded mip chain. Complex resources retain the metadata-preserving path.
    let base_image = if metadata.dimension == TEX_DIMENSION::TEX_DIMENSION_TEXTURE2D
        && !metadata.is_cubemap()
        && metadata.array_size == 1
        && metadata.depth <= 1
    {
        Some(
            scratch
                .image(0, 0, 0)
                .ok_or_else(|| "dds image does not contain a readable base mip".to_string())?,
        )
    } else {
        None
    };
    let rgba_scratch = if metadata.format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        None
    } else if metadata.format.is_compressed() {
        Some(match base_image {
            Some(image) => image
                .decompress(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM)
                .map_err(|err| err.to_string())?,
            None => scratch
                .decompress(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM)
                .map_err(|err| err.to_string())?,
        })
    } else {
        Some(match base_image {
            Some(image) => image
                .convert(
                    DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                    TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                    TEX_THRESHOLD_DEFAULT,
                )
                .map_err(|err| err.to_string())?,
            None => scratch
                .convert(
                    DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                    TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                    TEX_THRESHOLD_DEFAULT,
                )
                .map_err(|err| err.to_string())?,
        })
    };

    let image_owner = rgba_scratch.as_ref().unwrap_or(&scratch);
    let image = image_owner
        .image(0, 0, 0)
        .ok_or_else(|| "dds conversion did not produce an RGBA image".to_string())?;
    let rgba = packed_pixels_from_image(image, false, false)?;
    Ok((width, height, rgba, dxgi_format))
}

pub(crate) fn write_dds_bytes(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
) -> core::result::Result<(), String> {
    write_dds_bytes_with_compression(
        output_path,
        width,
        height,
        rgba,
        format,
        generate_mips,
        true,
        false,
    )
}

fn write_dds_bytes_with_compression(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
    parallel_compression: bool,
    use_gpu: bool,
) -> core::result::Result<(), String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Encode);
    let width_usize = usize::try_from(width).map_err(|_| "width does not fit usize".to_string())?;
    let height_usize =
        usize::try_from(height).map_err(|_| "height does not fit usize".to_string())?;
    let expected_len = width_usize
        .checked_mul(height_usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "rgba buffer size overflow".to_string())?;
    if rgba.len() != expected_len {
        return Err(format!(
            "rgba buffer length mismatch: expected {expected_len} bytes, got {}",
            rgba.len()
        ));
    }

    if let Some((target_format, fourcc, block_channels)) = legacy_dxt_target(format) {
        let chain: Vec<(u32, u32, Vec<u8>)> =
            rgba_mip_chain(width_usize, height_usize, rgba, generate_mips)?
                .into_iter()
                .map(|(w, h, px)| {
                    let w =
                        u32::try_from(w).map_err(|_| "mip width does not fit u32".to_string())?;
                    let h =
                        u32::try_from(h).map_err(|_| "mip height does not fit u32".to_string())?;
                    Ok((w, h, px))
                })
                .collect::<core::result::Result<_, String>>()?;
        let encoded = encode_legacy_dxt_dds_from_chain(
            &chain,
            target_format,
            fourcc,
            block_channels,
            parallel_compression,
        )?;
        if let Some(parent) = output_path.parent() {
            crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        return crate::profiling::write(output_path, encoded).map_err(|err| err.to_string());
    }

    let target_format =
        parse_dxgi_format(format).ok_or_else(|| format!("unsupported DDS format: {format}"))?;

    if matches!(
        target_format,
        DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
    ) {
        let encoded = encode_rgba8_dds(width, height, rgba, target_format, generate_mips)?;
        if let Some(parent) = output_path.parent() {
            crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        return crate::profiling::write(output_path, encoded).map_err(|err| err.to_string());
    }

    let native_bc = match target_format {
        DXGI_FORMAT::DXGI_FORMAT_BC4_UNORM => Some(1),
        DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM => Some(2),
        _ => None,
    };
    if let Some(channels) = native_bc {
        let encoded = encode_bc_unorm_dds(width, height, rgba, channels, generate_mips)?;
        if let Some(parent) = output_path.parent() {
            crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        return crate::profiling::write(output_path, encoded).map_err(|err| err.to_string());
    }

    let is_ispc_target = matches!(
        target_format,
        DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM_SRGB
            | DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM_SRGB
            | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB
    );
    if target_format.is_compressed() && (generate_mips || is_ispc_target) {
        let encoded = encode_compressed_dds(
            width,
            height,
            rgba,
            target_format,
            generate_mips,
            parallel_compression,
            use_gpu,
        )?;
        if let Some(parent) = output_path.parent() {
            crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        return crate::profiling::write(output_path, encoded).map_err(|err| err.to_string());
    }

    let mut scratch = ScratchImage::default();
    scratch
        .initialize_2d(
            DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
            width_usize,
            height_usize,
            1,
            1,
            CP_FLAGS::CP_FLAGS_NONE,
        )
        .map_err(|err| err.to_string())?;
    scratch.pixels_mut().copy_from_slice(rgba);

    let scratch = if generate_mips {
        scratch
            .generate_mip_maps(TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT, 0)
            .map_err(|err| err.to_string())?
    } else {
        scratch
    };

    let encoded = if target_format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        scratch.save_dds(DDS_FLAGS::DDS_FLAGS_NONE)
    } else if target_format.is_compressed() {
        let scratch = scratch_for_target_compression(scratch, target_format)?;
        scratch
            .compress(
                target_format,
                compression_flags_for_format(target_format, parallel_compression),
                TEX_THRESHOLD_DEFAULT,
            )
            .and_then(|compressed| compressed.save_dds(DDS_FLAGS::DDS_FLAGS_NONE))
    } else {
        scratch
            .convert(
                target_format,
                TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                TEX_THRESHOLD_DEFAULT,
            )
            .and_then(|converted| converted.save_dds(DDS_FLAGS::DDS_FLAGS_NONE))
    }
    .map_err(|err| err.to_string())?;

    if let Some(parent) = output_path.parent() {
        crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    crate::profiling::write(output_path, encoded.buffer()).map_err(|err| err.to_string())
}

pub fn read_dds_rgba_image(path: &Path) -> core::result::Result<DdsRgbaImage, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Decode);
    let (width, height, rgba, dxgi_format) = dds_base_rgba(path)?;
    Ok(DdsRgbaImage {
        width,
        height,
        rgba,
        dxgi_format,
    })
}

pub fn read_dds_rgba_image_bytes(bytes: &[u8]) -> core::result::Result<DdsRgbaImage, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Decode);
    let (width, height, rgba, dxgi_format) = dds_base_rgba_bytes(bytes)?;
    Ok(DdsRgbaImage {
        width,
        height,
        rgba,
        dxgi_format,
    })
}

pub fn write_dds_rgba_image(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
) -> core::result::Result<(), String> {
    write_dds_bytes(output_path, width, height, rgba, format, generate_mips)
}

fn encode_srgb_payload_with_unorm_header(
    chain: &[(u32, u32, Vec<u8>)],
    format: &str,
) -> core::result::Result<Vec<u8>, String> {
    let unorm_format =
        parse_dxgi_format(format).ok_or_else(|| format!("unsupported DDS format: {format}"))?;
    if unorm_format.is_srgb() {
        return Err(format!(
            "sRGB payload writer requires a UNORM output format, got {format}"
        ));
    }
    let srgb_format = unorm_format.make_srgb();
    if !srgb_format.is_srgb() {
        return Err(format!(
            "DDS format has no sRGB payload equivalent: {format}"
        ));
    }

    if unorm_format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        let converted_chain = chain
            .iter()
            .map(|(width, height, rgba)| {
                Ok((
                    *width,
                    *height,
                    convert_unorm_texels_to_srgb(*width as usize, *height as usize, rgba)?,
                ))
            })
            .collect::<core::result::Result<Vec<_>, String>>()?;
        return encode_dds_from_rgba8_chain(&converted_chain, format, true, None);
    }

    let srgb_format_name = dxgi_format_name(srgb_format);
    let mut encoded = encode_dds_from_rgba8_chain(chain, &srgb_format_name, true, None)?;
    if encoded.get(84..88) != Some(b"DX10") {
        return Err("sRGB payload DDS is missing a DX10 header".to_string());
    }
    let header_format = encoded
        .get_mut(128..132)
        .ok_or_else(|| "sRGB payload DDS has a truncated DX10 header".to_string())?;
    header_format.copy_from_slice(&unorm_format.bits().to_le_bytes());
    Ok(encoded)
}

fn validate_rgba8_buffer(width: u32, height: u32, rgba: &[u8]) -> core::result::Result<(), String> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "rgba buffer size overflow".to_string())?;
    if rgba.len() != expected_len {
        return Err(format!(
            "rgba buffer length mismatch: expected {expected_len} bytes, got {}",
            rgba.len()
        ));
    }
    Ok(())
}

fn write_srgb_payload_with_unorm_header(
    output_path: &Path,
    chain: &[(u32, u32, Vec<u8>)],
    format: &str,
) -> core::result::Result<(), String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Encode);
    let encoded = encode_srgb_payload_with_unorm_header(chain, format)?;
    if let Some(parent) = output_path.parent() {
        crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    crate::profiling::write(output_path, encoded).map_err(|err| err.to_string())
}

pub fn write_dds_rgba_image_srgb_payload_unorm_header(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
) -> core::result::Result<(), String> {
    validate_rgba8_buffer(width, height, rgba)?;
    let chain = if generate_mips {
        rgba8_box_mip_chain(width, height, rgba)?
    } else {
        vec![(width, height, rgba.to_vec())]
    };
    write_srgb_payload_with_unorm_header(output_path, &chain, format)
}

pub fn write_dds_rgba_image_mip_flooded(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
) -> core::result::Result<(), String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Encode);
    let chain = rgba8_mip_flood_chain(width, height, rgba)?;
    let output_format = mip_flood_output_format(format, rgba);
    let encoded = encode_dds_from_rgba8_chain(&chain, output_format, true, None)?;
    if let Some(parent) = output_path.parent() {
        crate::profiling::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    crate::profiling::write(output_path, encoded).map_err(|err| err.to_string())
}

pub fn write_dds_rgba_image_mip_flooded_srgb_payload_unorm_header(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
) -> core::result::Result<(), String> {
    validate_rgba8_buffer(width, height, rgba)?;
    let chain = rgba8_mip_flood_chain(width, height, rgba)?;
    let output_format = mip_flood_output_format(format, rgba);
    write_srgb_payload_with_unorm_header(output_path, &chain, output_format)
}

pub fn write_dds_rgba_image_gpu(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
    use_gpu: bool,
) -> core::result::Result<(), String> {
    write_dds_bytes_with_compression(
        output_path,
        width,
        height,
        rgba,
        format,
        generate_mips,
        true,
        use_gpu,
    )
}

fn load_dds_float_rgba(path: &Path) -> core::result::Result<(u32, u32, Vec<f32>, u32), String> {
    let (width, height, rgba, dxgi_format) = dds_base_rgba(path)?;
    let pixels = rgba.into_iter().map(|v| f32::from(v) / 255.0).collect();
    Ok((width, height, pixels, dxgi_format))
}

pub fn read_dds_float_rgba_image(path: &Path) -> core::result::Result<DdsRgbaFloatImage, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Decode);
    let (width, height, rgba, dxgi_format) = load_dds_float_rgba(path)?;
    Ok(DdsRgbaFloatImage {
        width,
        height,
        rgba,
        dxgi_format,
    })
}

/// Cheap DDS header probe — reads ≤ 148 bytes, never decodes pixels.
/// `dxgi_format == 0` means "could not identify" (caller must fall back to a
/// full load). Legacy-fourCC mapping mirrors DirectXTex's own (DXT1→BC1_UNORM
/// etc. — no sRGB variants exist in legacy headers).
#[derive(Debug, Clone, Copy)]
pub struct DdsProbe {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub dxgi_format: u32,
    pub is_cubemap: bool,
}

/// Full mip-chain length for a base level (floor(log2(max(w,h))) + 1).
pub fn full_mip_count(width: u32, height: u32) -> u32 {
    let m = width.max(height).max(1);
    32 - m.leading_zeros()
}

pub fn read_dds_probe(path: &Path) -> core::result::Result<DdsProbe, String> {
    use std::io::Read;
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 148];
    let mut filled = 0usize;
    while filled < buf.len() {
        match f.read(&mut buf[filled..]).map_err(|e| e.to_string())? {
            0 => break,
            n => filled += n,
        }
    }
    read_dds_probe_bytes(&buf[..filled])
}

pub fn read_dds_probe_bytes(bytes: &[u8]) -> core::result::Result<DdsProbe, String> {
    if bytes.len() < 128 || &bytes[0..4] != b"DDS " {
        return Err("not a DDS file".to_string());
    }
    let u32_at =
        |off: usize| -> u32 { u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) };
    let height = u32_at(12);
    let width = u32_at(16);
    let depth = u32_at(24).max(1);
    let mip_levels = u32_at(28).max(1);
    let pf_flags = u32_at(80);
    let fourcc = &bytes[84..88];
    let caps2 = u32_at(112);
    let mut is_cubemap = (caps2 & 0x200) != 0;
    let mut array_size = 1u32;
    let dxgi_format: u32;
    if fourcc == b"DX10" {
        if bytes.len() < 148 {
            return Err("truncated DX10 DDS header".to_string());
        }
        dxgi_format = u32_at(128);
        if (u32_at(136) & 0x4) != 0 {
            is_cubemap = true;
        }
        array_size = u32_at(140).max(1);
    } else {
        dxgi_format = match fourcc {
            b"DXT1" => 71,           // BC1_UNORM
            b"DXT2" | b"DXT3" => 74, // BC2_UNORM
            b"DXT4" | b"DXT5" => 77, // BC3_UNORM
            b"ATI1" | b"BC4U" => 80, // BC4_UNORM
            b"ATI2" | b"BC5U" => 83, // BC5_UNORM
            _ => {
                // Uncompressed RGB path: only claim the one layout we can prove.
                let rgb_bits = u32_at(88);
                let (r, g, b, a) = (u32_at(92), u32_at(96), u32_at(100), u32_at(104));
                if (pf_flags & 0x40) != 0
                    && rgb_bits == 32
                    && (r, g, b, a) == (0xff, 0xff00, 0xff_0000, 0xff00_0000)
                {
                    28 // R8G8B8A8_UNORM
                } else {
                    0 // unknown — caller falls back to a full load / residue class
                }
            }
        };
    }
    Ok(DdsProbe {
        width,
        height,
        depth,
        mip_levels,
        array_size,
        dxgi_format,
        is_cubemap,
    })
}

/// Every mip of array-item 0, decoded to RGBA8. BC5 uses the same custom
/// decoder as `dds_base_rgba` (B=0, A=255 fill) so per-texel kernels see the
/// identical pixel convention the legacy base-mip reader produces.
#[derive(Debug, Clone)]
pub struct DdsMipsRgba8 {
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub mips: Vec<(u32, u32, Vec<u8>)>,
}

impl DdsMipsRgba8 {
    pub fn into_base_float_image(self) -> core::result::Result<DdsRgbaFloatImage, String> {
        let (_, _, pixels) = self
            .mips
            .into_iter()
            .next()
            .ok_or_else(|| "empty mip chain".to_string())?;
        Ok(DdsRgbaFloatImage {
            width: self.width,
            height: self.height,
            dxgi_format: self.dxgi_format,
            rgba: pixels.into_iter().map(|v| f32::from(v) / 255.0).collect(),
        })
    }
}

pub fn read_dds_mips_rgba8(path: &Path) -> core::result::Result<DdsMipsRgba8, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Decode);
    let bytes = crate::profiling::read(path).map_err(|err| err.to_string())?;
    let scratch = ScratchImage::load_dds(&bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
        .map_err(|err| err.to_string())?;
    let metadata = *scratch.metadata();
    if metadata.is_cubemap() || metadata.array_size > 1 || metadata.depth > 1 {
        return Err("read_dds_mips_rgba8 supports plain 2D textures only".to_string());
    }
    let width: u32 = metadata
        .width
        .try_into()
        .map_err(|_| "width exceeds u32".to_string())?;
    let height: u32 = metadata
        .height
        .try_into()
        .map_err(|_| "height exceeds u32".to_string())?;
    let dxgi_format = metadata.format.bits();
    let mut mips = Vec::with_capacity(metadata.mip_levels);

    if metadata.format == DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM {
        for mip in 0..metadata.mip_levels {
            let image = scratch
                .image(mip, 0, 0)
                .ok_or_else(|| format!("missing BC5 mip {mip}"))?;
            let rgba = decode_bc5_unorm_image(image)?;
            mips.push((image.width as u32, image.height as u32, rgba));
        }
        return Ok(DdsMipsRgba8 {
            width,
            height,
            dxgi_format,
            mips,
        });
    }

    let rgba_scratch = if metadata.format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        None
    } else if metadata.format.is_compressed() {
        Some(
            scratch
                .decompress(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM)
                .map_err(|err| err.to_string())?,
        )
    } else {
        Some(
            scratch
                .convert(
                    DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                    TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                    TEX_THRESHOLD_DEFAULT,
                )
                .map_err(|err| err.to_string())?,
        )
    };
    let owner = rgba_scratch.as_ref().unwrap_or(&scratch);
    for mip in 0..metadata.mip_levels {
        let image = owner
            .image(mip, 0, 0)
            .ok_or_else(|| format!("missing mip {mip}"))?;
        let rgba = packed_pixels_from_image(image, false, false)?;
        mips.push((image.width as u32, image.height as u32, rgba));
    }
    Ok(DdsMipsRgba8 {
        width,
        height,
        dxgi_format,
        mips,
    })
}

/// The legacy box-filter mip chain (same `rgba_mip_chain` the writers use),
/// with u32 dimensions for engine callers.
pub fn rgba8_box_mip_chain(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> core::result::Result<Vec<(u32, u32, Vec<u8>)>, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Mips);
    let chain = rgba_mip_chain(width as usize, height as usize, rgba, true)?;
    Ok(chain
        .into_iter()
        .map(|(w, h, px)| (w as u32, h as u32, px))
        .collect())
}

/// Encode a caller-supplied RGBA8 mip chain into a complete DDS byte stream.
/// Byte-identical to `write_dds_rgba_image(..., generate_mips=true)` on the same
/// chain (pinned by `encode_dds_from_rgba8_chain_matches_legacy_writer_bytes`).
/// BC7 levels go through `bc7_encoder` (the engine's GpuService) when provided;
/// on encoder error or `None`, the per-level CPU path is used.
pub fn encode_dds_from_rgba8_chain(
    chain: &[(u32, u32, Vec<u8>)],
    format: &str,
    parallel_compression: bool,
    bc7_encoder: Option<
        &(dyn Fn(&[(u32, u32, &[u8])], bool) -> core::result::Result<Vec<Vec<u8>>, String> + Sync),
    >,
) -> core::result::Result<Vec<u8>, String> {
    let adapter = |chain: &[(u32, u32, Vec<u8>)], srgb| {
        let refs: Vec<_> = chain
            .iter()
            .map(|(w, h, pixels)| (*w, *h, pixels.as_slice()))
            .collect();
        bc7_encoder.expect("adapter only used with encoder")(&refs, srgb)
    };
    encode_dds_chain(
        chain,
        format,
        parallel_compression,
        bc7_encoder.map(|_| &adapter as _),
    )
}

pub type Rgba8MipChain = Vec<(u32, u32, Vec<u8>)>;
pub type SharedBc7Encoder<'a> = dyn Fn(std::sync::Arc<Rgba8MipChain>, bool) -> core::result::Result<Vec<Vec<u8>>, String>
    + Sync
    + 'a;

pub fn encode_dds_from_owned_rgba8_chain(
    chain: Rgba8MipChain,
    format: &str,
    parallel_compression: bool,
    bc7_encoder: Option<&SharedBc7Encoder<'_>>,
) -> core::result::Result<Vec<u8>, String> {
    // Retain the pixels for the existing CPU fallback while the GPU owns a
    // shared reference. Only the Arc is cloned, never the mip buffers.
    let chain = std::sync::Arc::new(chain);
    let adapter = |_: &[(u32, u32, Vec<u8>)], srgb| {
        bc7_encoder.expect("adapter only used with encoder")(chain.clone(), srgb)
    };
    encode_dds_chain(
        &chain,
        format,
        parallel_compression,
        bc7_encoder.map(|_| &adapter as _),
    )
}

fn encode_dds_chain(
    chain: &[(u32, u32, Vec<u8>)],
    format: &str,
    parallel_compression: bool,
    bc7_encoder: Option<
        &dyn Fn(&[(u32, u32, Vec<u8>)], bool) -> core::result::Result<Vec<Vec<u8>>, String>,
    >,
) -> core::result::Result<Vec<u8>, String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Encode);
    let (width, height) = match chain.first() {
        Some((w, h, _)) => (*w, *h),
        None => return Err("empty mip chain".to_string()),
    };
    for (w, h, px) in chain {
        let expected = (*w as usize) * (*h as usize) * 4;
        if px.len() != expected {
            return Err(format!("mip {w}x{h}: rgba len {} != {expected}", px.len()));
        }
    }
    if let Some((target_format, fourcc, block_channels)) = legacy_dxt_target(format) {
        return encode_legacy_dxt_dds_from_chain(
            chain,
            target_format,
            fourcc,
            block_channels,
            parallel_compression,
        );
    }
    let target_format =
        parse_dxgi_format(format).ok_or_else(|| format!("unsupported DDS format: {format}"))?;
    let mip_count =
        u32::try_from(chain.len()).map_err(|_| "mip count does not fit u32".to_string())?;

    // RGBA8 family — mirrors encode_rgba8_dds.
    if matches!(
        target_format,
        DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
    ) {
        let pitch = width
            .checked_mul(4)
            .ok_or_else(|| "dds pitch overflow".to_string())?;
        let mut out = if target_format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
            dds_rgba8_header(width, height, pitch, mip_count)
        } else {
            dds_dx10_header(width, height, pitch, target_format, mip_count)
        };
        for (_, _, px) in chain {
            out.extend_from_slice(px);
        }
        return Ok(out);
    }

    // BC4/BC5 — mirrors encode_bc_unorm_dds (ATI1/ATI2 headers).
    let native_bc = match target_format {
        DXGI_FORMAT::DXGI_FORMAT_BC4_UNORM => Some(1usize),
        DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM => Some(2usize),
        _ => None,
    };
    if let Some(channels) = native_bc {
        let linear_size = bc_level_size(width as usize, height as usize, channels)?;
        let fourcc = if channels == 1 { b"ATI1" } else { b"ATI2" };
        let mut out = dds_header(width, height, linear_size as u32, fourcc, mip_count);
        for (w, h, px) in chain {
            if channels == 2 {
                out.extend_from_slice(&ispc_bc::bc5_blocks_from_rgba(
                    *w as usize,
                    *h as usize,
                    px,
                )?);
            } else {
                append_bc_unorm_level(&mut out, *w as usize, *h as usize, px, channels)?;
            }
        }
        return Ok(out);
    }

    // Remaining compressed targets — mirrors encode_compressed_dds.
    if !target_format.is_compressed() {
        return Err(format!("unsupported chain format: {format}"));
    }
    let block_channels = if matches!(
        target_format,
        DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM_SRGB
    ) {
        1
    } else {
        2
    };
    let linear_size = bc_level_size(width as usize, height as usize, block_channels)?;
    let mut out = dds_dx10_header(width, height, linear_size as u32, target_format, mip_count);

    let is_bc7 = matches!(
        target_format,
        DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM | DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB
    );
    if is_bc7 {
        if let Some(encoder) = bc7_encoder {
            let srgb = target_format == DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB;
            if let Ok(payloads) = encoder(chain, srgb) {
                if payloads.len() == chain.len() {
                    for payload in payloads {
                        out.extend_from_slice(&payload);
                    }
                    return Ok(out);
                }
            }
            // fall through to the CPU per-level path on any encoder failure
        }
    }
    for (w, h, px) in chain {
        out.extend_from_slice(&compressed_payload_from_rgba(
            *w as usize,
            *h as usize,
            px,
            target_format,
            parallel_compression,
            false,
        )?);
    }
    Ok(out)
}

pub fn write_dds_float_rgba_image(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[f32],
    format: &str,
    generate_mips: bool,
) -> core::result::Result<(), String> {
    write_dds_float_rgba_image_with_compression(
        output_path,
        width,
        height,
        rgba,
        format,
        generate_mips,
        true,
        false,
    )
}

pub fn write_dds_float_rgba_image_gpu(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[f32],
    format: &str,
    generate_mips: bool,
    parallel_compression: bool,
    use_gpu: bool,
) -> core::result::Result<(), String> {
    write_dds_float_rgba_image_with_compression(
        output_path,
        width,
        height,
        rgba,
        format,
        generate_mips,
        parallel_compression,
        use_gpu,
    )
}

fn write_dds_float_rgba_image_with_compression(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[f32],
    format: &str,
    generate_mips: bool,
    parallel_compression: bool,
    use_gpu: bool,
) -> core::result::Result<(), String> {
    let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Encode);
    let bytes: Vec<u8> = rgba
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    write_dds_bytes_with_compression(
        output_path,
        width,
        height,
        &bytes,
        format,
        generate_mips,
        parallel_compression,
        use_gpu,
    )
}

#[cfg(test)]
fn write_dds_float_rgba(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[f32],
    format: &str,
) -> core::result::Result<(), String> {
    write_dds_float_rgba_with_compression(output_path, width, height, rgba, format, false, true)
}

fn write_dds_float_rgba_with_compression(
    output_path: &Path,
    width: u32,
    height: u32,
    rgba: &[f32],
    format: &str,
    generate_mips: bool,
    parallel_compression: bool,
) -> core::result::Result<(), String> {
    write_dds_float_rgba_image_with_compression(
        output_path,
        width,
        height,
        rgba,
        format,
        generate_mips,
        parallel_compression,
        false,
    )
}

fn resize_rgba_float(
    rgba: Vec<f32>,
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
) -> core::result::Result<Vec<f32>, String> {
    if (src_width, src_height) == (dst_width, dst_height) {
        return Ok(rgba);
    }
    let src_width_usize =
        usize::try_from(src_width).map_err(|_| "source width does not fit usize".to_string())?;
    let src_height_usize =
        usize::try_from(src_height).map_err(|_| "source height does not fit usize".to_string())?;
    let dst_width_usize =
        usize::try_from(dst_width).map_err(|_| "target width does not fit usize".to_string())?;
    let dst_height_usize =
        usize::try_from(dst_height).map_err(|_| "target height does not fit usize".to_string())?;
    if src_width_usize == 0
        || src_height_usize == 0
        || dst_width_usize == 0
        || dst_height_usize == 0
    {
        return Err("cannot resize zero-sized texture".to_string());
    }
    let expected_src = src_width_usize
        .checked_mul(src_height_usize)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| "source texture dimensions overflow usize".to_string())?;
    if rgba.len() != expected_src {
        return Err("source texture data length does not match dimensions".to_string());
    }
    let out_len = dst_width_usize
        .checked_mul(dst_height_usize)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| "target texture dimensions overflow usize".to_string())?;
    let mut out = vec![0.0; out_len];
    let scale_x = if dst_width_usize > 1 {
        (src_width_usize - 1) as f32 / (dst_width_usize - 1) as f32
    } else {
        0.0
    };
    let scale_y = if dst_height_usize > 1 {
        (src_height_usize - 1) as f32 / (dst_height_usize - 1) as f32
    } else {
        0.0
    };

    for y in 0..dst_height_usize {
        let sy = y as f32 * scale_y;
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(src_height_usize - 1);
        let fy = sy - y0 as f32;
        for x in 0..dst_width_usize {
            let sx = x as f32 * scale_x;
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(src_width_usize - 1);
            let fx = sx - x0 as f32;
            for channel in 0..4 {
                let a = rgba[(y0 * src_width_usize + x0) * 4 + channel];
                let b = rgba[(y0 * src_width_usize + x1) * 4 + channel];
                let c = rgba[(y1 * src_width_usize + x0) * 4 + channel];
                let d = rgba[(y1 * src_width_usize + x1) * 4 + channel];
                let top = a + (b - a) * fx;
                let bottom = c + (d - c) * fx;
                out[(y * dst_width_usize + x) * 4 + channel] = top + (bottom - top) * fy;
            }
        }
    }
    Ok(out)
}

fn remix_fo76_pixel_texture(
    rgba: &mut [f32],
    role: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
) {
    for px in rgba.chunks_exact_mut(4) {
        match role {
            "n" => {
                let nx = px[0] * 2.0 - 1.0;
                let ny = px[1] * 2.0 - 1.0;
                let nz = (1.0 - nx * nx - ny * ny).clamp(0.0, 1.0).sqrt();
                px[2] = (nz + 1.0) * 0.5;
            }
            "r" => {
                let roughness = px[0];
                let metallic = px[1];
                let ao = px[2];
                let albedo = roughness;
                let spec = ((0.04 + (albedo - 0.04) * metallic) * specular_multiplier
                    + spec_offset)
                    .clamp(0.0, 1.0);
                let gloss = ((1.0 - roughness) * gloss_multiplier).clamp(0.0, 1.0);
                let _diffuse = albedo * (1.0 - metallic) * ao * ao_multiplier;
                px[0] = spec;
                px[1] = spec;
                px[2] = spec;
                px[3] = gloss;
            }
            "l" => {
                let ao = (px[0] * ao_multiplier).clamp(0.0, 1.0);
                px[0] = ao;
                px[1] = ao;
                px[2] = ao;
            }
            _ => {}
        }
    }
}

pub fn remix_fo76_texture_bytes(
    src_path: &Path,
    dst_path: &Path,
    role: &str,
    format: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
) -> core::result::Result<(), String> {
    remix_fo76_texture_bytes_with_compression(
        src_path,
        dst_path,
        role,
        format,
        ao_multiplier,
        specular_multiplier,
        gloss_multiplier,
        spec_offset,
        true,
    )
}

pub fn remix_fo76_texture_bytes_with_compression(
    src_path: &Path,
    dst_path: &Path,
    role: &str,
    format: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
    parallel_compression: bool,
) -> core::result::Result<(), String> {
    let (width, height, mut rgba, _) = load_dds_float_rgba(src_path)?;
    remix_fo76_pixel_texture(
        &mut rgba,
        role,
        ao_multiplier,
        specular_multiplier,
        gloss_multiplier,
        spec_offset,
    );
    write_dds_float_rgba_with_compression(
        dst_path,
        width,
        height,
        &rgba,
        format,
        // FO4 landscape (and all FO4) textures require a full mip chain; a
        // single-mip terrain texture fails to stream and renders the quad black.
        true,
        parallel_compression,
    )
}

pub fn remix_fo76_bundle_bytes(
    diffuse_path: &Path,
    reflectivity_path: &Path,
    lighting_path: &Path,
    diffuse_out_path: &Path,
    specgloss_out_path: &Path,
    glow_out_path: &Path,
    diffuse_format: &str,
    specgloss_format: &str,
    glow_format: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
) -> core::result::Result<(), String> {
    remix_fo76_bundle_bytes_with_compression(
        diffuse_path,
        reflectivity_path,
        lighting_path,
        diffuse_out_path,
        specgloss_out_path,
        glow_out_path,
        diffuse_format,
        specgloss_format,
        glow_format,
        ao_multiplier,
        specular_multiplier,
        gloss_multiplier,
        spec_offset,
        true,
    )
}

pub fn remix_fo76_bundle_bytes_with_compression(
    diffuse_path: &Path,
    reflectivity_path: &Path,
    lighting_path: &Path,
    diffuse_out_path: &Path,
    specgloss_out_path: &Path,
    glow_out_path: &Path,
    diffuse_format: &str,
    specgloss_format: &str,
    glow_format: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
    parallel_compression: bool,
) -> core::result::Result<(), String> {
    let (width, height, diffuse, _) = load_dds_float_rgba(diffuse_path)?;
    let (rw, rh, reflectivity, _) = load_dds_float_rgba(reflectivity_path)?;
    let (lw, lh, lighting, _) = load_dds_float_rgba(lighting_path)?;
    let reflectivity = resize_rgba_float(reflectivity, rw, rh, width, height)?;
    let lighting = resize_rgba_float(lighting, lw, lh, width, height)?;

    let mut diffuse_out = diffuse.clone();
    let mut specgloss = vec![0.0; diffuse.len()];
    let mut glow = vec![0.0; diffuse.len()];
    let threshold = 1.0 - spec_offset;
    let denom = spec_offset.max(1e-6);

    for idx in 0..(diffuse.len() / 4) {
        let i = idx * 4;
        let d0 = diffuse[i].clamp(0.0, 1.0);
        let d1 = diffuse[i + 1].clamp(0.0, 1.0);
        let d2 = diffuse[i + 2].clamp(0.0, 1.0);
        let r0 = reflectivity[i].clamp(0.0, 1.0);
        let r1 = reflectivity[i + 1].clamp(0.0, 1.0);
        let r2 = reflectivity[i + 2].clamp(0.0, 1.0);
        let ao = lighting[i + 1].clamp(0.0, 1.0);
        let gloss = (lighting[i].clamp(0.0, 1.0) * gloss_multiplier).clamp(0.0, 1.0);

        let [metal0, metal1, metal2] =
            metal_contribution_preserving_hue(r0, r1, r2, threshold, denom);

        let base0 = (d0 + metal0).clamp(0.0, 1.0);
        let base1 = (d1 + metal1).clamp(0.0, 1.0);
        let base2 = (d2 + metal2).clamp(0.0, 1.0);
        let ao_term = (1.0 - ao_multiplier) + ao * ao_multiplier;

        diffuse_out[i] = (base0 * ao_term).clamp(0.0, 1.0);
        diffuse_out[i + 1] = (base1 * ao_term).clamp(0.0, 1.0);
        diffuse_out[i + 2] = (base2 * ao_term).clamp(0.0, 1.0);

        let spec0 = 0.22 * (1.0 - metal0) + base0 * metal0;
        let spec1 = 0.22 * (1.0 - metal1) + base1 * metal1;
        let spec2 = 0.22 * (1.0 - metal2) + base2 * metal2;
        let specular = (((spec0 + spec1 + spec2) / 3.0) * specular_multiplier).clamp(0.0, 1.0);

        specgloss[i] = specular;
        specgloss[i + 1] = gloss;
        specgloss[i + 2] = 0.0;
        specgloss[i + 3] = 1.0;

        let emissive = lighting[i + 3].clamp(0.0, 1.0);
        glow[i] = emissive;
        glow[i + 1] = emissive;
        glow[i + 2] = emissive;
        glow[i + 3] = 1.0;
    }

    write_dds_float_rgba_with_compression(
        diffuse_out_path,
        width,
        height,
        &diffuse_out,
        diffuse_format,
        true,
        parallel_compression,
    )?;
    write_dds_float_rgba_with_compression(
        specgloss_out_path,
        width,
        height,
        &specgloss,
        specgloss_format,
        true,
        parallel_compression,
    )?;
    write_dds_float_rgba_with_compression(
        glow_out_path,
        width,
        height,
        &glow,
        glow_format,
        true,
        parallel_compression,
    )
}

fn metal_contribution_preserving_hue(
    reflectivity_r: f32,
    reflectivity_g: f32,
    reflectivity_b: f32,
    threshold: f32,
    denominator: f32,
) -> [f32; 3] {
    let peak = reflectivity_r.max(reflectivity_g).max(reflectivity_b);
    let remapped_peak = ((peak - threshold).max(0.0) / denominator).clamp(0.0, 1.0);
    if peak <= 0.0 {
        return [remapped_peak; 3];
    }

    // spec_offset filters reflectivity magnitude. Applying its threshold to
    // each channel separately destroys colored-metal hue (gold/copper turn red).
    let scale = remapped_peak / peak;
    [
        reflectivity_r * scale,
        reflectivity_g * scale,
        reflectivity_b * scale,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_rgb_dds(width: u32, height: u32, bits: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&0x40u32.to_le_bytes());
        bytes[88..92].copy_from_slice(&bits.to_le_bytes());
        for (offset, mask) in [92usize, 96, 100, 104].into_iter().zip(masks) {
            bytes[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn sniff_dds_checks_size_legacy_format_and_optional_sse_masks() {
        let bytes = legacy_rgb_dds(300, 256, 24, [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0]);
        let report = validate_dds_bytes(&bytes, false).unwrap();
        assert_eq!(report.width, 300);
        assert_eq!(report.height, 256);
        assert_eq!(report.findings.len(), 2);
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.rule == "invalid-texture-size-format")
        );

        let reduced = legacy_rgb_dds(256, 256, 16, [0xF800, 0x07E0, 0x001F, 0]);
        assert!(
            validate_dds_bytes(&reduced, false)
                .unwrap()
                .findings
                .is_empty()
        );
        assert_eq!(
            validate_dds_bytes(&reduced, true).unwrap().findings[0].rule,
            "sse-unsupported-texture-format"
        );
    }

    #[test]
    fn sniff_dds_rejects_invalid_header() {
        assert_eq!(
            validate_dds_bytes(b"not a dds", false).unwrap_err(),
            "Not a valid DDS file"
        );
    }

    #[test]
    fn metal_contribution_preserves_reflectivity_hue() {
        let actual = metal_contribution_preserving_hue(0.4, 0.25, 0.1, 0.2, 0.8);
        let expected = [0.25, 0.15625, 0.0625];

        for channel in 0..3 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 1e-6,
                "channel {channel}: expected {}, got {}",
                expected[channel],
                actual[channel]
            );
        }

        for actual in metal_contribution_preserving_hue(0.25, 0.25, 0.25, 0.2, 0.8) {
            assert!((actual - 0.0625).abs() < 1e-6);
        }
    }

    #[test]
    fn bc7_uses_quick_parallel_compression_flags() {
        let flags = compression_flags_for_format(DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM, true);

        assert!(flags.contains(TEX_COMPRESS_FLAGS::TEX_COMPRESS_BC7_QUICK));
        assert!(flags.contains(TEX_COMPRESS_FLAGS::TEX_COMPRESS_PARALLEL));
    }

    #[test]
    fn bc7_can_disable_parallel_compression_flags() {
        let flags = compression_flags_for_format(DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM, false);

        assert!(flags.contains(TEX_COMPRESS_FLAGS::TEX_COMPRESS_BC7_QUICK));
        assert!(!flags.contains(TEX_COMPRESS_FLAGS::TEX_COMPRESS_PARALLEL));
    }

    #[test]
    fn opaque_bc1_and_bc3_route_through_ispc() {
        let width = 6usize;
        let height = 10usize;
        let mut rgba = vec![0u8; width * height * 4];
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[
                (index * 17) as u8,
                (index * 31) as u8,
                (index * 47) as u8,
                255,
            ]);
        }

        let routed_bc1 = compressed_payload_from_rgba(
            width,
            height,
            &rgba,
            DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM,
            false,
            false,
        )
        .unwrap();
        let routed_bc3 = compressed_payload_from_rgba(
            width,
            height,
            &rgba,
            DXGI_FORMAT::DXGI_FORMAT_BC3_UNORM,
            false,
            false,
        )
        .unwrap();

        assert_eq!(
            routed_bc1,
            ispc_bc::bc1_blocks_from_rgba(width, height, &rgba).unwrap()
        );
        assert_eq!(
            routed_bc3,
            ispc_bc::bc3_blocks_from_rgba(width, height, &rgba).unwrap()
        );
    }

    #[test]
    fn bc1_cutout_alpha_keeps_directxtex_fallback() {
        let width = 8usize;
        let height = 8usize;
        let mut rgba = vec![40u8, 180, 70, 255].repeat(width * height);
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            if index % 2 == 0 {
                pixel[3] = 0;
            }
        }

        let routed = compressed_payload_from_rgba(
            width,
            height,
            &rgba,
            DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM,
            false,
            false,
        )
        .unwrap();
        let directxtex = dxtex_compressed_payload(
            width,
            height,
            &rgba,
            DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM,
            false,
        )
        .unwrap();

        assert_eq!(routed, directxtex);
    }

    #[test]
    fn no_mip_bc1_and_bc3_writers_use_ispc_payloads() {
        let width = 8usize;
        let height = 8usize;
        let rgba = vec![40u8, 180, 70, 255].repeat(width * height);
        let tmp =
            std::env::temp_dir().join(format!("modbox21_ispc_bc1_bc3_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        for (format, filename, expected_payload) in [
            (
                "BC1_UNORM",
                "bc1.dds",
                ispc_bc::bc1_blocks_from_rgba(width, height, &rgba).unwrap(),
            ),
            (
                "BC3_UNORM",
                "bc3.dds",
                ispc_bc::bc3_blocks_from_rgba(width, height, &rgba).unwrap(),
            ),
        ] {
            let path = tmp.join(filename);
            write_dds_rgba_image(&path, width as u32, height as u32, &rgba, format, false).unwrap();
            let bytes = fs::read(path).unwrap();
            assert_eq!(&bytes[84..88], b"DX10");
            assert_eq!(&bytes[148..], expected_payload);
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn public_rgba_helpers_roundtrip_dds() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("modbox21_public_rgba_{}.dds", std::process::id()));
        let width = 2u32;
        let height = 2u32;
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];

        write_dds_rgba_image(&path, width, height, &rgba, "R8G8B8A8_UNORM", false).unwrap();

        let loaded = read_dds_rgba_image(&path).unwrap();
        assert_eq!(loaded.width, width);
        assert_eq!(loaded.height, height);
        assert_eq!(loaded.rgba, rgba);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn mip_flood_chain_fills_rgb_and_preserves_base_alpha() {
        let mut rgba = vec![0u8; 4 * 4 * 4];
        let center = (2 * 4 + 2) * 4;
        rgba[center..center + 4].copy_from_slice(&[220, 40, 10, 128]);
        let original_alpha: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();

        let chain = rgba8_mip_flood_chain(4, 4, &rgba).unwrap();

        assert_eq!(
            chain.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(),
            vec![(4, 4), (2, 2), (1, 1)]
        );
        let base = &chain[0].2;
        assert!(
            base.chunks_exact(4)
                .all(|pixel| pixel[..3] == [220, 40, 10])
        );
        assert_eq!(
            base.chunks_exact(4)
                .map(|pixel| pixel[3])
                .collect::<Vec<_>>(),
            original_alpha
        );
    }

    #[test]
    fn mip_flood_downsample_ignores_rgb_with_zero_alpha() {
        let rgba = vec![255, 0, 0, 255, 0, 0, 255, 0];

        let chain = rgba8_mip_flood_chain(2, 1, &rgba).unwrap();

        assert_eq!(chain[1].2, vec![255, 0, 0, 128]);
        assert_eq!(&chain[0].2[4..7], &[255, 0, 0]);
        assert_eq!(chain[0].2[7], 0);
    }

    #[test]
    fn mip_flood_fully_transparent_image_is_unchanged() {
        let rgba = vec![10, 20, 30, 0, 40, 50, 60, 0];

        let chain = rgba8_mip_flood_chain(2, 1, &rgba).unwrap();

        assert_eq!(chain[0].2, rgba);
        assert_eq!(chain[1].2, vec![25, 35, 45, 0]);
    }

    #[test]
    fn mip_flood_keeps_sub_byte_alpha_coverage_as_a_color_seed() {
        let mut rgba = vec![0u8; 16 * 16 * 4];
        rgba[..4].copy_from_slice(&[20, 180, 60, 1]);

        let chain = rgba8_mip_flood_chain(16, 16, &rgba).unwrap();

        assert_eq!(&chain.last().unwrap().2[..3], &[20, 180, 60]);
        assert!(
            chain[0]
                .2
                .chunks_exact(4)
                .all(|pixel| pixel[..3] == [20, 180, 60])
        );
        assert_eq!(chain[0].2[3], 1);
        assert_eq!(chain[0].2[7], 0);
    }

    #[test]
    fn mip_flood_preserves_standard_alpha_at_every_level() {
        let mut rgba = vec![0u8; 8 * 8 * 4];
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[80, 120, 160, ((index * 37) % 256) as u8]);
        }

        let flooded = rgba8_mip_flood_chain(8, 8, &rgba).unwrap();
        let standard = rgba8_box_mip_chain(8, 8, &rgba).unwrap();

        assert_eq!(flooded.len(), standard.len());
        for (flooded_level, standard_level) in flooded.iter().zip(&standard) {
            assert_eq!(
                (flooded_level.0, flooded_level.1),
                (standard_level.0, standard_level.1)
            );
            assert_eq!(
                flooded_level
                    .2
                    .chunks_exact(4)
                    .map(|pixel| pixel[3])
                    .collect::<Vec<_>>(),
                standard_level
                    .2
                    .chunks_exact(4)
                    .map(|pixel| pixel[3])
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn mip_flood_generates_standard_mips_from_flooded_base() {
        let mut rgba = vec![0u8; 8 * 8 * 4];
        rgba[..4].copy_from_slice(&[220, 30, 10, 255]);
        let last = rgba.len() - 4;
        rgba[last..].copy_from_slice(&[10, 40, 230, 128]);

        let flooded = rgba8_mip_flood_chain(8, 8, &rgba).unwrap();
        let standard_from_flooded = rgba8_box_mip_chain(8, 8, &flooded[0].2).unwrap();

        assert_eq!(flooded, standard_from_flooded);
    }

    #[test]
    fn mip_flood_writer_promotes_bc1_cutout_to_bc3() {
        let path = std::env::temp_dir().join(format!(
            "modbox21_mip_flood_bc1_cutout_{}.dds",
            std::process::id()
        ));
        let mut rgba = vec![0u8; 8 * 8 * 4];
        rgba[..4].copy_from_slice(&[40, 180, 70, 255]);

        write_dds_rgba_image_mip_flooded(&path, 8, 8, &rgba, "BC1_UNORM_SRGB").unwrap();

        let decoded = read_dds_mips_rgba8(&path).unwrap();
        assert_eq!(decoded.dxgi_format, 78);
        assert!(
            decoded.mips[0]
                .2
                .chunks_exact(4)
                .any(|pixel| { pixel[3] == 0 && pixel[..3].iter().any(|channel| *channel != 0) })
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn mip_flood_writer_emits_direct_dds_mip_chain() {
        let path = std::env::temp_dir().join(format!(
            "modbox21_mip_flood_writer_{}.dds",
            std::process::id()
        ));
        let mut rgba = vec![0u8; 4 * 4 * 4];
        rgba[..4].copy_from_slice(&[12, 80, 200, 255]);

        write_dds_rgba_image_mip_flooded(&path, 4, 4, &rgba, "R8G8B8A8_UNORM").unwrap();
        let decoded = read_dds_mips_rgba8(&path).unwrap();

        assert_eq!(decoded.mips.len(), 3);
        assert_eq!(decoded.mips[0].2[3], 255);
        assert_eq!(decoded.mips[0].2[7], 0);
        assert_eq!(&decoded.mips[0].2[4..7], &[12, 80, 200]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn srgb_texels_convert_to_linear_unorm() {
        let converted = convert_srgb_texels_to_linear_unorm(1, 1, &[128, 64, 32, 200]).unwrap();

        assert!((converted[0] as i32 - 55).abs() <= 1);
        assert!((converted[1] as i32 - 13).abs() <= 1);
        assert!((converted[2] as i32 - 4).abs() <= 1);
        assert_eq!(converted[3], 200);
    }

    #[test]
    fn srgb_payload_writer_keeps_unorm_header_and_builds_mips_in_linear_space() {
        let path = std::env::temp_dir().join(format!(
            "modbox21_srgb_payload_unorm_header_{}.dds",
            std::process::id()
        ));
        let rgba = vec![255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];

        write_dds_rgba_image_srgb_payload_unorm_header(&path, 2, 2, &rgba, "BC3_UNORM", true)
            .unwrap();

        let decoded = read_dds_mips_rgba8(&path).unwrap();
        assert_eq!(decoded.dxgi_format, 77);
        assert_eq!(decoded.mips.len(), 2);
        let last = &decoded.mips[1].2;
        for channel in &last[..3] {
            assert!(
                (120..=150).contains(channel),
                "linear 25% gray should encode near sRGB 137, got {channel}"
            );
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn bc5_unorm_can_write_full_mip_chain() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("modbox21_bc5_mips_{}.dds", std::process::id()));
        let width = 8;
        let height = 8;
        let rgba = vec![128u8; width * height * 4];

        write_dds_bytes(&path, width as u32, height as u32, &rgba, "BC5_UNORM", true).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(&bytes[0..4], b"DDS ");
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 4);
        assert_eq!(bytes.len(), 128 + 64 + 16 + 16 + 16);
    }

    #[test]
    fn bc5_writer_uses_ispc_payload() {
        let path =
            std::env::temp_dir().join(format!("modbox21_ispc_bc5_{}.dds", std::process::id()));
        let width = 6usize;
        let height = 10usize;
        let mut rgba = vec![0u8; width * height * 4];
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[
                (index * 17) as u8,
                (index * 31) as u8,
                (index * 47) as u8,
                255,
            ]);
        }

        write_dds_rgba_image(
            &path,
            width as u32,
            height as u32,
            &rgba,
            "BC5_UNORM",
            false,
        )
        .unwrap();
        let bytes = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(&bytes[84..88], b"ATI2");
        assert_eq!(
            &bytes[128..],
            ispc_bc::bc5_blocks_from_rgba(width, height, &rgba).unwrap()
        );
    }

    #[test]
    fn legacy_dxt_formats_write_fourcc_headers() {
        let tmp = std::env::temp_dir().join(format!("modbox21_legacy_dxt_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let rgba = vec![200u8, 100, 50, 255].repeat(16 * 16);

        for (format, fourcc, expected_dxgi, expected_len) in [
            ("DXT1", b"DXT1", 71u32, 128 + 128 + 32 + 8 + 8 + 8),
            ("DXT5", b"DXT5", 77u32, 128 + 256 + 64 + 16 + 16 + 16),
        ] {
            let path = tmp.join(format!("{format}.dds"));
            write_dds_rgba_image(&path, 16, 16, &rgba, format, true).unwrap();
            let bytes = fs::read(&path).unwrap();

            assert_eq!(&bytes[0..4], b"DDS ");
            assert_eq!(&bytes[84..88], fourcc, "{format}");
            assert_ne!(&bytes[84..88], b"DX10", "{format}");
            assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 5);
            assert_eq!(bytes.len(), expected_len, "{format}");

            let info = crate::dds_base_rgba(&path).unwrap();
            assert_eq!((info.0, info.1, info.3), (16, 16, expected_dxgi));
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_bc7_with_gpu_flag_produces_valid_dds() {
        let tmp = std::env::temp_dir().join("write_bc7_with_gpu_flag");
        let _ = std::fs::create_dir_all(&tmp);
        let path = tmp.join("foo_d.dds");
        let rgba = vec![200u8, 100, 50, 255].repeat(8 * 8);
        // use_gpu=true must succeed whether or not a GPU exists (CPU fallback).
        write_dds_rgba_image_gpu(&path, 8, 8, &rgba, "BC7_UNORM", true, true).unwrap();
        // Re-load it to confirm it's a valid BC7 DDS of the right dimensions.
        let info = crate::dds_base_rgba(&path).unwrap();
        assert_eq!((info.0, info.1), (8, 8));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn compressed_srgb_formats_write_valid_dds() {
        let tmp = std::env::temp_dir().join("modbox21_compressed_srgb_formats");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let rgba = vec![200u8, 100, 50, 255].repeat(8 * 8);
        let cases = [
            ("BC1_UNORM_SRGB", 72u32),
            ("BC3_UNORM_SRGB", 78u32),
            ("BC7_UNORM_SRGB", 99u32),
        ];

        for (format, expected_dxgi) in cases {
            for generate_mips in [false, true] {
                let path = tmp.join(format!("{format}_{generate_mips}.dds"));
                write_dds_rgba_image(&path, 8, 8, &rgba, format, generate_mips).unwrap();
                let info = crate::dds_base_rgba(&path).unwrap();
                assert_eq!(info.3, expected_dxgi, "{format} mips={generate_mips}");
            }
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rgba8_srgb_writes_valid_dds() {
        let tmp = std::env::temp_dir().join("modbox21_rgba8_srgb_format");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let rgba = vec![200u8, 100, 50, 255].repeat(8 * 8);

        for generate_mips in [false, true] {
            let path = tmp.join(format!("rgba8_srgb_{generate_mips}.dds"));
            write_dds_rgba_image(&path, 8, 8, &rgba, "R8G8B8A8_UNORM_SRGB", generate_mips).unwrap();
            let info = crate::dds_base_rgba(&path).unwrap();
            assert_eq!(info.3, 29, "mips={generate_mips}");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fo76_bundle_remix_resamples_auxiliary_maps_to_diffuse_dimensions() {
        let dir = std::env::temp_dir().join(format!(
            "modbox21_fo76_bundle_resample_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let diffuse = dir.join("diffuse.dds");
        let reflectivity = dir.join("reflectivity.dds");
        let lighting = dir.join("lighting.dds");
        let diffuse_out = dir.join("out_d.dds");
        let specgloss_out = dir.join("out_s.dds");
        let glow_out = dir.join("out_g.dds");

        write_dds_float_rgba(&diffuse, 8, 8, &vec![0.5; 8 * 8 * 4], "R8G8B8A8_UNORM").unwrap();
        write_dds_float_rgba(
            &reflectivity,
            4,
            4,
            &vec![0.25; 4 * 4 * 4],
            "R8G8B8A8_UNORM",
        )
        .unwrap();
        write_dds_float_rgba(&lighting, 8, 8, &vec![0.75; 8 * 8 * 4], "R8G8B8A8_UNORM").unwrap();

        remix_fo76_bundle_bytes(
            &diffuse,
            &reflectivity,
            &lighting,
            &diffuse_out,
            &specgloss_out,
            &glow_out,
            "R8G8B8A8_UNORM",
            "R8G8B8A8_UNORM",
            "R8G8B8A8_UNORM",
            1.0,
            1.0,
            1.0,
            0.04,
        )
        .unwrap();

        let bytes = std::fs::read(&specgloss_out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 8);
    }

    #[test]
    fn dds_probe_matches_full_metadata_for_written_formats() {
        let tmp = std::env::temp_dir().join(format!("dxt_probe_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let rgba = vec![128u8; 16 * 8 * 4];
        for (format, expect_dxgi) in [
            ("DXT1", 71u32),
            ("DXT5", 77u32),
            ("BC1_UNORM", 71u32),
            ("BC1_UNORM_SRGB", 72),
            ("BC3_UNORM", 77),
            ("BC4_UNORM", 80),
            ("BC5_UNORM", 83),
            ("BC7_UNORM", 98),
            ("BC7_UNORM_SRGB", 99),
            ("R8G8B8A8_UNORM", 28),
        ] {
            for mips in [false, true] {
                let p = tmp.join(format!("{format}_{mips}.dds"));
                write_dds_rgba_image(&p, 16, 8, &rgba, format, mips).unwrap();
                let probe = read_dds_probe(&p).unwrap();
                assert_eq!(probe.width, 16, "{format}");
                assert_eq!(probe.height, 8, "{format}");
                assert_eq!(probe.dxgi_format, expect_dxgi, "{format} mips={mips}");
                assert_eq!(
                    probe.mip_levels,
                    if mips { 5 } else { 1 },
                    "{format} mips={mips}"
                );
                assert!(!probe.is_cubemap);
                assert_eq!(probe.array_size, 1);
            }
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dds_probe_full_mip_helper() {
        assert_eq!(full_mip_count(1, 1), 1);
        assert_eq!(full_mip_count(16, 8), 5);
        assert_eq!(full_mip_count(1024, 512), 11);
        assert_eq!(full_mip_count(4096, 4096), 13);
    }

    #[test]
    fn read_dds_mips_rgba8_returns_every_level() {
        let tmp = std::env::temp_dir().join(format!("dxt_mips_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let rgba: Vec<u8> = (0..16usize * 16 * 4).map(|i| (i % 251) as u8).collect();
        for format in ["R8G8B8A8_UNORM", "BC1_UNORM", "BC5_UNORM", "BC7_UNORM"] {
            let p = tmp.join(format!("{format}.dds"));
            write_dds_rgba_image(&p, 16, 16, &rgba, format, true).unwrap();
            let chain = read_dds_mips_rgba8(&p).unwrap();
            assert_eq!(chain.mips.len(), 5, "{format}");
            assert_eq!((chain.mips[0].0, chain.mips[0].1), (16, 16));
            assert_eq!((chain.mips[4].0, chain.mips[4].1), (1, 1));
            for (w, h, px) in &chain.mips {
                assert_eq!(px.len(), (*w as usize) * (*h as usize) * 4, "{format}");
            }
            if format == "BC5_UNORM" {
                // BC5 fill convention must match dds_base_rgba (B=0, A=255).
                assert!(
                    chain.mips[0]
                        .2
                        .chunks_exact(4)
                        .all(|p| p[2] == 0 && p[3] == 255)
                );
            }
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn encode_dds_from_rgba8_chain_matches_legacy_writer_bytes() {
        let tmp = std::env::temp_dir().join(format!("dxt_chain_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let rgba: Vec<u8> = (0..32usize * 16 * 4).map(|i| (i * 7 % 256) as u8).collect();
        for format in [
            "R8G8B8A8_UNORM",
            "R8G8B8A8_UNORM_SRGB",
            "DXT1",
            "DXT5",
            "BC1_UNORM",
            "BC4_UNORM",
            "BC5_UNORM",
            "BC7_UNORM",
            "BC7_UNORM_SRGB",
        ] {
            let p = tmp.join(format!("legacy_{format}.dds"));
            // Legacy writer: generate_mips=true, parallel=true, use_gpu=false.
            write_dds_rgba_image(&p, 32, 16, &rgba, format, true).unwrap();
            let legacy = fs::read(&p).unwrap();
            let chain = rgba8_box_mip_chain(32, 16, &rgba).unwrap();
            let ours = encode_dds_from_rgba8_chain(&chain, format, true, None).unwrap();
            assert_eq!(
                ours, legacy,
                "{format}: chain encoder must byte-match legacy"
            );
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn encode_dds_from_rgba8_chain_uses_external_bc7_encoder() {
        let chain = rgba8_box_mip_chain(8, 8, &vec![200u8; 8 * 8 * 4]).unwrap();
        let called = std::sync::atomic::AtomicUsize::new(0);
        let enc = |imgs: &[(u32, u32, &[u8])], srgb: bool| {
            called.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(srgb);
            imgs.iter()
                .map(|(w, h, px)| {
                    compressed_payload_from_rgba(
                        *w as usize,
                        *h as usize,
                        px,
                        DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB,
                        false,
                        false,
                    )
                })
                .collect()
        };
        let out = encode_dds_from_rgba8_chain(&chain, "BC7_UNORM_SRGB", false, Some(&enc)).unwrap();
        assert_eq!(called.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Must equal the no-encoder CPU output (the closure delegates to the same CPU encode).
        let cpu = encode_dds_from_rgba8_chain(&chain, "BC7_UNORM_SRGB", false, None).unwrap();
        assert_eq!(out, cpu);
    }

    #[test]
    fn batch_gpu_fn_is_publicly_reachable() {
        // Compile-time check that the re-export exists; runtime tolerates no GPU.
        let _ = crate::compress_bc7_gpu_batch(&[], false);
    }
}

#[cfg(test)]
mod owned_chain_tests;

#[cfg(test)]
mod write_bench;
