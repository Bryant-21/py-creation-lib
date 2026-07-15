use terrain_native::height_resample::{
    LANCZOS2_REACH, SourceGrid, TargetGrid, resample_feature, resample_lanczos, resample_sample4,
    resample_weighted,
};
use terrain_native::land_encode::{decode_vhgt_heights, encode_vhgt, generate_vnml};

fn synthetic_source(f: impl Fn(usize, usize) -> f32) -> SourceGrid {
    let mut values = Vec::with_capacity(128 * 128);
    for y in 0..128 {
        for x in 0..128 {
            values.push(f(x, y));
        }
    }
    SourceGrid {
        width: 128,
        height: 128,
        values,
    }
}

fn diagonal_ridge_source() -> SourceGrid {
    // Crest along x - y = 2, deliberately off the 4-sample target lattice.
    synthetic_source(|x, y| 4096.0 - ((x as f32 - y as f32) - 2.0).abs() * 24.0)
}

fn sine_hills_source() -> SourceGrid {
    use std::f32::consts::TAU;
    synthetic_source(|x, y| {
        let x = x as f32;
        let y = y as f32;
        2048.0
            + 1024.0 * (x * (TAU / 37.0)).sin() * (y * (TAU / 41.0)).cos()
            + 256.0 * (x * (TAU / 5.3)).sin() * (y * (TAU / 6.1)).cos()
    })
}

fn step_cliff_source() -> SourceGrid {
    // Cliff edge at x = 53, not a multiple of 4.
    synthetic_source(|x, _y| if x >= 53 { 1024.0 } else { 0.0 })
}

fn bilinear_reconstruct(grid: &TargetGrid, width: usize, height: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let fy = y as f32 / 4.0;
        let y0 = (fy.floor() as usize).min(grid.height - 1);
        let y1 = (y0 + 1).min(grid.height - 1);
        let wy = fy - y0 as f32;
        for x in 0..width {
            let fx = x as f32 / 4.0;
            let x0 = (fx.floor() as usize).min(grid.width - 1);
            let x1 = (x0 + 1).min(grid.width - 1);
            let wx = fx - x0 as f32;
            let top = grid.get(x0, y0) + (grid.get(x1, y0) - grid.get(x0, y0)) * wx;
            let bottom = grid.get(x0, y1) + (grid.get(x1, y1) - grid.get(x0, y1)) * wx;
            out.push(top + (bottom - top) * wy);
        }
    }
    out
}

fn reconstruction_rms(source: &SourceGrid, grid: &TargetGrid) -> f64 {
    let recon = bilinear_reconstruct(grid, source.width, source.height);
    let sse: f64 = source
        .values
        .iter()
        .zip(&recon)
        .map(|(a, b)| {
            let diff = (*a - *b) as f64;
            diff * diff
        })
        .sum();
    (sse / source.values.len() as f64).sqrt()
}

fn kernel_footprint_range(source: &SourceGrid, tx: usize, ty: usize) -> (f32, f32) {
    let center_x = (tx * 4) as i64;
    let center_y = (ty * 4) as i64;
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;
    for dy in -LANCZOS2_REACH..=LANCZOS2_REACH {
        let y = (center_y + dy as i64).clamp(0, source.height as i64 - 1) as usize;
        for dx in -LANCZOS2_REACH..=LANCZOS2_REACH {
            let x = (center_x + dx as i64).clamp(0, source.width as i64 - 1) as usize;
            let value = source.values[y * source.width + x];
            min_value = min_value.min(value);
            max_value = max_value.max(value);
        }
    }
    (min_value, max_value)
}

#[test]
fn lanczos_reconstructs_better_than_sample4_on_synthetic_terrain() {
    for (name, source) in [
        ("diagonal_ridge", diagonal_ridge_source()),
        ("sine_hills", sine_hills_source()),
        ("step_cliff", step_cliff_source()),
    ] {
        let sample4 = resample_sample4(&source, 1, 1).unwrap();
        let weighted = resample_weighted(&source, 1, 1).unwrap();
        let lanczos = resample_lanczos(&source, 1, 1).unwrap();
        let sample4_rms = reconstruction_rms(&source, &sample4);
        let weighted_rms = reconstruction_rms(&source, &weighted);
        let lanczos_rms = reconstruction_rms(&source, &lanczos);
        eprintln!(
            "{name}: sample4={sample4_rms:.3} weighted={weighted_rms:.3} lanczos={lanczos_rms:.3}"
        );
        assert!(
            lanczos_rms < sample4_rms,
            "{name}: lanczos rms {lanczos_rms} not below sample4 rms {sample4_rms}"
        );
    }
}

#[test]
fn lanczos_output_stays_within_kernel_footprint_bounds() {
    for source in [
        diagonal_ridge_source(),
        sine_hills_source(),
        step_cliff_source(),
    ] {
        let out = resample_lanczos(&source, 1, 1).unwrap();
        for ty in 0..out.height {
            for tx in 0..out.width {
                let (min_value, max_value) = kernel_footprint_range(&source, tx, ty);
                let value = out.get(tx, ty);
                assert!(
                    value >= min_value && value <= max_value,
                    "({tx}, {ty}): {value} outside footprint [{min_value}, {max_value}]"
                );
            }
        }
    }
}

#[test]
fn sample4_maps_128_samples_to_33_vertices() {
    let mut values = Vec::new();
    for y in 0..128 {
        for x in 0..128 {
            values.push((x as f32 * 2.0) + (y as f32 * 3.0));
        }
    }
    let source = SourceGrid {
        width: 128,
        height: 128,
        values,
    };
    let out = resample_sample4(&source, 1, 1).unwrap();

    assert_eq!(out.width, 33);
    assert_eq!(out.height, 33);
    assert_eq!(out.get(0, 0), 0.0);
    assert_eq!(out.get(32, 32), (127.0 * 2.0) + (127.0 * 3.0));
}

#[test]
fn weighted_resampler_clamps_to_source_footprint() {
    let mut values = vec![100.0; 128 * 128];
    values[64 * 128 + 64] = 900.0;
    let source = SourceGrid {
        width: 128,
        height: 128,
        values,
    };

    let out = resample_weighted(&source, 1, 1).unwrap();

    for value in out.values {
        assert!((100.0..=900.0).contains(&value));
    }
}

#[test]
fn feature_resampler_preserves_isolated_peak_better_than_weighted() {
    let mut values = vec![100.0; 128 * 128];
    values[64 * 128 + 64] = 900.0;
    let source = SourceGrid {
        width: 128,
        height: 128,
        values,
    };

    let weighted = resample_weighted(&source, 1, 1).unwrap();
    let feature = resample_feature(&source, 1, 1).unwrap();

    assert!(feature.get(16, 16) > weighted.get(16, 16) + 200.0);
    assert!(feature.get(16, 16) >= 800.0);
}

#[test]
fn resampler_rejects_invalid_source_dimensions_without_panicking() {
    let source = SourceGrid {
        width: usize::MAX,
        height: 2,
        values: Vec::new(),
    };

    assert!(resample_sample4(&source, 1, 1).is_err());
}

#[test]
fn resampler_rejects_oversized_output_dimensions_without_allocation() {
    let source = SourceGrid {
        width: 1,
        height: 1,
        values: vec![0.0],
    };

    assert!(resample_sample4(&source, usize::MAX / 32, 2).is_err());
}

#[test]
fn resampler_rejects_output_capacity_above_vec_limit_without_panicking() {
    let source = SourceGrid {
        width: 1,
        height: 1,
        values: vec![0.0],
    };
    let max_vec_elements = isize::MAX as usize / std::mem::size_of::<f32>();
    let target_width = (max_vec_elements / 33) + 1;
    let cells_x = target_width.div_ceil(32);

    let result = std::panic::catch_unwind(|| resample_sample4(&source, cells_x, 1));

    assert!(result.is_ok());
    assert!(result.unwrap().is_err());
}

#[test]
fn vhgt_roundtrip_preserves_shared_edge_values() {
    let mut heights = Vec::new();
    for y in 0..33 {
        for x in 0..33 {
            heights.push(1024.0 + (x as f32 * 8.0) + (y as f32 * 8.0));
        }
    }

    let encoded = encode_vhgt(&heights).unwrap();
    let decoded = decode_vhgt_heights(&encoded).unwrap();

    assert_eq!(encoded.raw.len(), 4 + 1089 + 3);
    assert_eq!(decoded.len(), 1089);
    assert_eq!(decoded[0], 1024.0);
    assert_eq!(decoded[32], 1280.0);
}

#[test]
fn vhgt_quantizes_first_height_to_lattice_offset() {
    // CK rounds the VHGT offset to an integer multiple of HEIGHT_STEP on
    // save. encode_vhgt now matches that — 1025.0/8 = 128.125 → 128.0.
    let heights = vec![1025.0; 33 * 33];

    let encoded = encode_vhgt(&heights).unwrap();
    let decoded = decode_vhgt_heights(&encoded).unwrap();

    assert_eq!(encoded.offset, 128.0);
    assert_eq!(encoded.raw.len(), 4 + 1089 + 3);
    assert_eq!(decoded[0], 1024.0);
}

#[test]
fn vhgt_encoder_rejects_negative_128_delta_for_ck_compatibility() {
    let mut heights = vec![0.0; 33 * 33];
    heights[1] = -128.0 * 8.0;

    assert!(encode_vhgt(&heights).is_err());
}

#[test]
fn vhgt_row_deltas_reset_to_previous_row_start() {
    let mut heights = vec![1024.0; 33 * 33];
    for x in 0..33 {
        heights[x] = 1024.0 + x as f32 * 127.0 * 8.0;
    }
    heights[33] = 1024.0 + 8.0;

    let encoded = encode_vhgt(&heights).unwrap();
    let decoded = decode_vhgt_heights(&encoded).unwrap();

    assert_eq!(decoded[0], 1024.0);
    assert_eq!(decoded[32], 1024.0 + 32.0 * 127.0 * 8.0);
    assert_eq!(decoded[33], 1024.0 + 8.0);
}

#[test]
fn generated_normals_are_nonzero() {
    let heights = vec![256.0; 33 * 33];
    let normals = generate_vnml(&heights);

    assert_eq!(normals.len(), 33 * 33 * 3);
    assert!(normals.chunks_exact(3).all(|n| n != [0, 0, 0]));
}

#[test]
fn generated_normals_use_land_vertex_spacing_for_moderate_slope() {
    let mut heights = Vec::new();
    for _y in 0..33 {
        for x in 0..33 {
            heights.push(256.0 + x as f32 * 8.0);
        }
    }

    let normals = generate_vnml(&heights);
    let center = (16 * 33 + 16) * 3;

    // Signed-i8 encoding: nx ≈ -0.062 → ≈ -8 → 0xF8; ny ≈ 0 → 0x00; nz ≈ 1 → 0x7F.
    let nx = normals[center] as i8;
    let ny = normals[center + 1] as i8;
    let nz = normals[center + 2] as i8;
    assert!((-12..=-4).contains(&nx), "nx out of range: {}", nx);
    assert!((-3..=3).contains(&ny), "ny out of range: {}", ny);
    assert!(nz >= 125, "nz too small: {}", nz);
}
