//! CPU BC block encoding via Intel's ISPC texture compressor (`intel_tex_2`).

use intel_tex_2::{RgSurface, RgbaSurface, bc1, bc3, bc5, bc7};

/// ISPC kernels require width/height multiples of 4 and read whole 4x4 blocks.
/// DDS stores ceil(w/4)*ceil(h/4) blocks, so pad odd dimensions and mip tails
/// by edge replication.
fn edge_padded(width: usize, height: usize, rgba: &[u8]) -> (usize, usize, Vec<u8>) {
    let padded_w = width.div_ceil(4) * 4;
    let padded_h = height.div_ceil(4) * 4;
    let mut padded = vec![0u8; padded_w * padded_h * 4];
    for y in 0..padded_h {
        let sy = y.min(height - 1);
        let src_row = &rgba[sy * width * 4..][..width * 4];
        let dst_row = &mut padded[y * padded_w * 4..][..padded_w * 4];
        dst_row[..width * 4].copy_from_slice(src_row);
        let last: [u8; 4] = src_row[width * 4 - 4..].try_into().expect("4-byte texel");
        for x in width..padded_w {
            dst_row[x * 4..x * 4 + 4].copy_from_slice(&last);
        }
    }
    (padded_w, padded_h, padded)
}

fn blocks_from_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
    format: &str,
    compress: impl FnOnce(&RgbaSurface<'_>) -> Vec<u8>,
) -> core::result::Result<Vec<u8>, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "rgba buffer size overflow".to_string())?;
    if rgba.len() < expected || width == 0 || height == 0 {
        return Err(format!(
            "{format} input {width}x{height} needs {expected} bytes, got {}",
            rgba.len()
        ));
    }
    if width % 4 == 0 && height % 4 == 0 {
        let surface = RgbaSurface {
            data: &rgba[..expected],
            width: width as u32,
            height: height as u32,
            stride: (width * 4) as u32,
        };
        return Ok(compress(&surface));
    }
    let (padded_w, padded_h, padded) = edge_padded(width, height, rgba);
    let surface = RgbaSurface {
        data: &padded,
        width: padded_w as u32,
        height: padded_h as u32,
        stride: (padded_w * 4) as u32,
    };
    Ok(compress(&surface))
}

pub(crate) fn bc1_blocks_from_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
) -> core::result::Result<Vec<u8>, String> {
    blocks_from_rgba(width, height, rgba, "bc1", bc1::compress_blocks)
}

pub(crate) fn bc3_blocks_from_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
) -> core::result::Result<Vec<u8>, String> {
    blocks_from_rgba(width, height, rgba, "bc3", bc3::compress_blocks)
}

pub(crate) fn bc5_blocks_from_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
) -> core::result::Result<Vec<u8>, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "rgba buffer size overflow".to_string())?;
    if rgba.len() < expected || width == 0 || height == 0 {
        return Err(format!(
            "bc5 input {width}x{height} needs {expected} bytes, got {}",
            rgba.len()
        ));
    }

    let padded_w = width.div_ceil(4) * 4;
    let padded_h = height.div_ceil(4) * 4;
    let mut rg = vec![0u8; padded_w * padded_h * 2];
    for y in 0..padded_h {
        let source_y = y.min(height - 1);
        for x in 0..padded_w {
            let source_x = x.min(width - 1);
            let source = (source_y * width + source_x) * 4;
            let destination = (y * padded_w + x) * 2;
            rg[destination..destination + 2].copy_from_slice(&rgba[source..source + 2]);
        }
    }
    let surface = RgSurface {
        data: &rg,
        width: padded_w as u32,
        height: padded_h as u32,
        stride: (padded_w * 2) as u32,
    };
    Ok(bc5::compress_blocks(&surface))
}

pub(crate) fn bc7_blocks_from_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
    settings: &bc7::EncodeSettings,
) -> core::result::Result<Vec<u8>, String> {
    blocks_from_rgba(width, height, rgba, "bc7", |surface| {
        bc7::compress_blocks(settings, surface)
    })
}

/// Alpha-aware profile: the opaque profiles ignore alpha entirely, so they
/// are only safe when every texel is fully opaque.
pub(crate) fn bc7_production_settings(rgba: &[u8]) -> bc7::EncodeSettings {
    if rgba.chunks_exact(4).all(|px| px[3] == 0xFF) {
        bc7::opaque_basic_settings()
    } else {
        bc7::alpha_basic_settings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkerboard(width: usize, height: usize, alpha: u8) -> Vec<u8> {
        let mut rgba = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let v = if (x + y) % 2 == 0 { 220 } else { 40 };
                let i = (y * width + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[v, 255 - v, v / 2, alpha]);
            }
        }
        rgba
    }

    #[test]
    fn block_counts_match_dds_expectations_for_all_mip_sizes() {
        for (w, h) in [(1, 1), (2, 2), (4, 4), (6, 10), (16, 8), (5, 3)] {
            let rgba = checkerboard(w, h, 255);
            let bc1 = bc1_blocks_from_rgba(w, h, &rgba).expect("bc1 encode");
            let bc3 = bc3_blocks_from_rgba(w, h, &rgba).expect("bc3 encode");
            let bc5 = bc5_blocks_from_rgba(w, h, &rgba).expect("bc5 encode");
            let bc7 = bc7_blocks_from_rgba(w, h, &rgba, &bc7::opaque_basic_settings())
                .expect("bc7 encode");
            assert_eq!(bc1.len(), w.div_ceil(4) * h.div_ceil(4) * 8);
            assert_eq!(bc3.len(), w.div_ceil(4) * h.div_ceil(4) * 16);
            assert_eq!(bc5.len(), w.div_ceil(4) * h.div_ceil(4) * 16);
            assert_eq!(bc7.len(), w.div_ceil(4) * h.div_ceil(4) * 16);
        }
    }

    #[test]
    fn bc7_profile_selection_is_alpha_keyed() {
        let opaque = checkerboard(4, 4, 255);
        let translucent = checkerboard(4, 4, 128);
        assert_eq!(bc7_production_settings(&opaque).channels, 3);
        assert_eq!(bc7_production_settings(&translucent).channels, 4);
    }
}
