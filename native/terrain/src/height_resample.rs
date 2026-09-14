use std::sync::OnceLock;

pub const LANCZOS2_REACH: isize = 7;
pub const LANCZOS2_TAPS: usize = 15;

#[derive(Debug, Clone)]
pub struct SourceGrid {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct TargetGrid {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl TargetGrid {
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.values[y * self.width + x]
    }
}

pub fn resample_sample4(
    source: &SourceGrid,
    cells_x: usize,
    cells_y: usize,
) -> Result<TargetGrid, String> {
    let (width, height, output_len) = validate_resample_inputs(source, cells_x, cells_y)?;
    let mut values = Vec::with_capacity(output_len);

    for ty in 0..height {
        for tx in 0..width {
            let sx = tx.saturating_mul(4).min(source.width - 1);
            let sy = ty.saturating_mul(4).min(source.height - 1);
            values.push(source.values[sy * source.width + sx]);
        }
    }

    Ok(TargetGrid {
        width,
        height,
        values,
    })
}

pub fn resample_weighted(
    source: &SourceGrid,
    cells_x: usize,
    cells_y: usize,
) -> Result<TargetGrid, String> {
    let (width, height, output_len) = validate_resample_inputs(source, cells_x, cells_y)?;
    let mut values = Vec::with_capacity(output_len);

    for ty in 0..height {
        for tx in 0..width {
            values.push(weighted_sample(source, tx, ty));
        }
    }

    Ok(TargetGrid {
        width,
        height,
        values,
    })
}

pub fn resample_feature(
    source: &SourceGrid,
    cells_x: usize,
    cells_y: usize,
) -> Result<TargetGrid, String> {
    let (width, height, output_len) = validate_resample_inputs(source, cells_x, cells_y)?;
    let mut values = Vec::with_capacity(output_len);

    for ty in 0..height {
        for tx in 0..width {
            values.push(feature_sample(source, tx, ty));
        }
    }

    Ok(TargetGrid {
        width,
        height,
        values,
    })
}

pub fn resample_lanczos(
    source: &SourceGrid,
    cells_x: usize,
    cells_y: usize,
) -> Result<TargetGrid, String> {
    let (width, height, output_len) = validate_resample_inputs(source, cells_x, cells_y)?;
    let kernel = lanczos2_kernel();
    let mut values = Vec::with_capacity(output_len);

    for ty in 0..height {
        for tx in 0..width {
            values.push(lanczos_sample(source, tx, ty, kernel));
        }
    }

    Ok(TargetGrid {
        width,
        height,
        values,
    })
}

fn validate_resample_inputs(
    source: &SourceGrid,
    cells_x: usize,
    cells_y: usize,
) -> Result<(usize, usize, usize), String> {
    if source.width == 0 || source.height == 0 {
        return Err("source grid dimensions must be non-zero".to_string());
    }
    let source_len = source
        .width
        .checked_mul(source.height)
        .ok_or_else(|| "source grid dimensions overflow usize".to_string())?;
    if source.values.len() != source_len {
        return Err("source grid values length does not match dimensions".to_string());
    }
    if cells_x == 0 || cells_y == 0 {
        return Err("output cell dimensions must be non-zero".to_string());
    }

    let width = cells_x
        .checked_mul(32)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "output width overflows usize".to_string())?;
    let height = cells_y
        .checked_mul(32)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "output height overflows usize".to_string())?;
    let output_len = width
        .checked_mul(height)
        .ok_or_else(|| "output grid dimensions overflow usize".to_string())?;
    let max_vec_elements = isize::MAX as usize / std::mem::size_of::<f32>();
    if output_len > max_vec_elements {
        return Err("output grid dimensions exceed Vec allocation limit".to_string());
    }

    Ok((width, height, output_len))
}

fn weighted_sample(source: &SourceGrid, tx: usize, ty: usize) -> f32 {
    let center_x = tx.saturating_mul(4) as f32;
    let center_y = ty.saturating_mul(4) as f32;

    let min_x = clamp_floor(center_x - 1.0, source.width);
    let max_x = clamp_ceil(center_x + 2.0, source.width);
    let min_y = clamp_floor(center_y - 1.0, source.height);
    let max_y = clamp_ceil(center_y + 2.0, source.height);

    let mut footprint = Vec::new();
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let value = source.values[y * source.width + x];
            min_value = min_value.min(value);
            max_value = max_value.max(value);
            footprint.push((x, y, value));
        }
    }

    let mean = footprint.iter().map(|(_, _, value)| *value).sum::<f32>() / footprint.len() as f32;
    let variance = footprint
        .iter()
        .map(|(_, _, value)| {
            let diff = *value - mean;
            diff * diff
        })
        .sum::<f32>()
        / footprint.len() as f32;
    let stddev = variance.sqrt();

    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;

    for (x, y, value) in footprint {
        let dx = x as f32 - center_x;
        let dy = y as f32 - center_y;
        let distance = (dx * dx + dy * dy).sqrt();
        let mut weight = 1.0 / (distance + 1.0);

        if stddev > 0.0 && (value - mean).abs() > stddev {
            weight *= 1.5;
        }

        weighted_sum += value * weight;
        total_weight += weight;
    }

    (weighted_sum / total_weight).clamp(min_value, max_value)
}

fn feature_sample(source: &SourceGrid, tx: usize, ty: usize) -> f32 {
    let center_x = tx.saturating_mul(4) as f32;
    let center_y = ty.saturating_mul(4) as f32;

    let min_x = clamp_floor(center_x - 2.0, source.width);
    let max_x = clamp_ceil(center_x + 2.0, source.width);
    let min_y = clamp_floor(center_y - 2.0, source.height);
    let max_y = clamp_ceil(center_y + 2.0, source.height);

    let mut count = 0usize;
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;
    let mut min_distance = f32::INFINITY;
    let mut max_distance = f32::INFINITY;
    let mut sum = 0.0f32;
    let mut weighted_sum = 0.0f32;
    let mut total_weight = 0.0f32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let value = source.values[y * source.width + x];
            let dx = x as f32 - center_x;
            let dy = y as f32 - center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            let weight = 1.0 / (distance + 1.0);
            if value < min_value {
                min_value = value;
                min_distance = distance;
            }
            if value > max_value {
                max_value = value;
                max_distance = distance;
            }
            sum += value;
            weighted_sum += value * weight;
            total_weight += weight;
            count += 1;
        }
    }

    let mean = sum / count as f32;
    let weighted = weighted_sum / total_weight;
    let relief = max_value - min_value;
    let high_centrality = 1.0 / (max_distance + 1.0);
    let low_centrality = 1.0 / (min_distance + 1.0);
    let (feature, centrality) =
        if (max_value - mean) * high_centrality >= (mean - min_value) * low_centrality {
            (max_value, high_centrality)
        } else {
            (min_value, low_centrality)
        };
    let blend = ((relief - 64.0) / 512.0).clamp(0.0, 1.0) * centrality;

    (weighted * (1.0 - blend) + feature * blend).clamp(min_value, max_value)
}

fn lanczos_sample(source: &SourceGrid, tx: usize, ty: usize, kernel: &[f32; LANCZOS2_TAPS]) -> f32 {
    let center_x = tx.saturating_mul(4);
    let center_y = ty.saturating_mul(4);
    let mut weighted_sum = 0.0f64;
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;

    for (ky, wy) in kernel.iter().enumerate() {
        let y = clamp_offset_index(center_y, ky as isize - LANCZOS2_REACH, source.height);
        for (kx, wx) in kernel.iter().enumerate() {
            let x = clamp_offset_index(center_x, kx as isize - LANCZOS2_REACH, source.width);
            let value = source.values[y * source.width + x];
            min_value = min_value.min(value);
            max_value = max_value.max(value);
            weighted_sum += (wx * wy) as f64 * value as f64;
        }
    }

    // Clamp to the footprint range so negative-lobe ringing cannot overshoot
    // past what the VHGT delta encode can represent.
    (weighted_sum as f32).clamp(min_value, max_value)
}

pub(crate) fn clamp_offset_index(center: usize, offset: isize, extent: usize) -> usize {
    (center as i64 + offset as i64).clamp(0, extent as i64 - 1) as usize
}

pub(crate) fn lanczos2_kernel() -> &'static [f32; LANCZOS2_TAPS] {
    static KERNEL: OnceLock<[f32; LANCZOS2_TAPS]> = OnceLock::new();
    KERNEL.get_or_init(|| {
        let mut weights = [0.0f64; LANCZOS2_TAPS];
        for (i, weight) in weights.iter_mut().enumerate() {
            *weight = lanczos2((i as f64 - LANCZOS2_REACH as f64) / 4.0);
        }
        let total: f64 = weights.iter().sum();
        let mut normalized = [0.0f32; LANCZOS2_TAPS];
        for (out, weight) in normalized.iter_mut().zip(weights) {
            *out = (weight / total) as f32;
        }
        normalized
    })
}

fn lanczos2(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let pix = std::f64::consts::PI * x;
    let half = pix / 2.0;
    (pix.sin() / pix) * (half.sin() / half)
}

fn clamp_floor(value: f32, extent: usize) -> usize {
    value.floor().clamp(0.0, (extent - 1) as f32) as usize
}

fn clamp_ceil(value: f32, extent: usize) -> usize {
    value.ceil().clamp(0.0, (extent - 1) as f32) as usize
}

pub const SF_LANCZOS2_MAX_TAPS: usize = 12;

#[derive(Clone, Debug)]
pub struct AxisTap {
    pub first: i32,
    pub count: u8,
    pub weights: [f32; SF_LANCZOS2_MAX_TAPS],
}

/// Lanczos-2 taps around a fractional `center` (source samples 1 apart), support
/// widened by `ratio` for downsampling: index `d` is included iff
/// `|d - center| < 2.0 * ratio`. The strict `<` drops the exact-zero boundary
/// taps, as `lanczos2_kernel()` does at its fixed ratio of 4. Weights sum to 1.0.
///
/// No reference kernel exists at the production ratio
/// (`SF_SAMPLES_PER_FO4_INTERVAL` ≈ 2.3409). The only external check is the
/// `starfield_heights_land_in_fo4_units` test in `authoring_emit.rs` (Akila peak
/// and span vs the BTD's per-cell min/max table); don't loosen it without
/// another way to verify this function.
fn lanczos2_taps_unbounded(center: f64, ratio: f64) -> (i32, Vec<f64>) {
    let radius = 2.0 * ratio;
    let first = (center - radius).floor() as i32 + 1;
    let last = (center + radius).ceil() as i32 - 1;
    let raw: Vec<f64> = (first..=last)
        .map(|d| lanczos2((d as f64 - center) / ratio))
        .collect();
    let total: f64 = raw.iter().sum();
    (first, raw.into_iter().map(|w| w / total).collect())
}

/// `lanczos2_taps_unbounded` capped to `SF_LANCZOS2_MAX_TAPS` slots; the
/// production ratio (`SF_SAMPLES_PER_FO4_INTERVAL`) needs at most 10. Needing
/// more panics instead of truncating: dropping even the smallest tap and
/// renormalizing shifts every weight far past any resample tolerance.
///
/// `assert!` is safe only because production always passes that constant ratio.
/// This runs under `convert_btd_to_authoring` across PyO3, where a panic aborts
/// the whole regen run instead of the pipeline's per-record fail-soft, so a
/// runtime-variable ratio needs a new `Result`-returning wrapper.
pub fn lanczos2_taps_at(center: f64, ratio: f64) -> AxisTap {
    let (first, normalized) = lanczos2_taps_unbounded(center, ratio);
    let count = normalized.len();
    assert!(
        count <= SF_LANCZOS2_MAX_TAPS,
        "lanczos2_taps_at: center {center} ratio {ratio} needs {count} taps, exceeding \
         SF_LANCZOS2_MAX_TAPS ({SF_LANCZOS2_MAX_TAPS})"
    );
    let mut weights = [0.0f32; SF_LANCZOS2_MAX_TAPS];
    for (slot, w) in weights.iter_mut().zip(&normalized) {
        *slot = *w as f32;
    }
    AxisTap {
        first,
        count: count as u8,
        weights,
    }
}

pub struct AxisTaps {
    pub taps: Vec<AxisTap>,
}

/// One `AxisTap` per target LAND vertex `0..vertex_count`, addressing BTD
/// source samples via `sf_frame::fo4_land_vertex_units` /
/// `sf_frame::fo4_units_to_btd_sample`.
pub fn build_axis_taps(
    vertex_count: usize,
    target_cell_min: i32,
    btd_cell_min: i32,
    ratio: f64,
) -> AxisTaps {
    let taps = (0..vertex_count)
        .map(|v| {
            let units = crate::sf_frame::fo4_land_vertex_units(target_cell_min, v);
            let center = crate::sf_frame::fo4_units_to_btd_sample(units, btd_cell_min);
            lanczos2_taps_at(center, ratio)
        })
        .collect();
    AxisTaps { taps }
}

#[cfg(test)]
mod sf_lanczos_tests {
    use super::*;

    #[test]
    fn lanczos2_taps_at_sums_to_one_and_fits_cap() {
        for i in 0..200 {
            let center = i as f64 * 0.037 - 3.0;
            let tap = lanczos2_taps_at(center, crate::sf_frame::SF_SAMPLES_PER_FO4_INTERVAL);
            assert!(tap.count as usize <= SF_LANCZOS2_MAX_TAPS);
            let sum: f32 = tap.weights[..tap.count as usize].iter().sum();
            assert!((sum - 1.0).abs() < 1e-6, "center {center} sum {sum}");
        }
    }

    /// Checked on the uncapped function: ratio 4.0 needs 15 taps, more than
    /// `SF_LANCZOS2_MAX_TAPS` (12) holds, and dropping even the smallest tap
    /// shifts the other weights by ~2e-3.
    #[test]
    fn lanczos2_taps_unbounded_reproduces_shipped_fo76_kernel_at_ratio_four() {
        let kernel = lanczos2_kernel();
        for &center in &[0.0, 5.0, -3.0] {
            let (first, weights) = lanczos2_taps_unbounded(center, 4.0);
            assert_eq!(weights.len(), LANCZOS2_TAPS);
            assert_eq!(first, center as i32 - LANCZOS2_REACH as i32);
            for (i, w) in weights.iter().enumerate() {
                assert!(
                    (*w as f32 - kernel[i]).abs() < 1e-6,
                    "tap {i}: unbounded {w} vs shipped {}",
                    kernel[i]
                );
            }
        }
    }

    #[test]
    fn lanczos2_taps_at_matches_unbounded_reference_in_production_range() {
        // Over a 40k-phase sweep, ratio 3.0 needs up to 12 taps (exactly
        // SF_LANCZOS2_MAX_TAPS) and 3.25 needs 13. It sits on that boundary on
        // purpose: if the support test in `lanczos2_taps_unbounded` changes from
        // `<` to `<=`, the cap assert in `lanczos2_taps_at` fails this test.
        let ratios = [
            crate::sf_frame::SF_SAMPLES_PER_FO4_INTERVAL,
            1.5,
            2.0,
            2.5,
            3.0,
        ];
        for &ratio in &ratios {
            for i in 0..200 {
                let center = i as f64 * 0.031 - 3.0;
                let capped = lanczos2_taps_at(center, ratio);
                let (first, weights) = lanczos2_taps_unbounded(center, ratio);
                assert_eq!(capped.first, first);
                assert_eq!(capped.count as usize, weights.len());
                for (i, w) in weights.iter().enumerate() {
                    assert!((capped.weights[i] - *w as f32).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn twelve_taps_suffice_for_every_reachable_ratio() {
        let mut ratio = 1.0f64;
        while ratio <= 3.0 {
            for i in 0..100 {
                let center = i as f64 * 0.071 - 5.0;
                let (_, weights) = lanczos2_taps_unbounded(center, ratio);
                assert!(
                    weights.len() <= SF_LANCZOS2_MAX_TAPS,
                    "ratio {ratio} center {center} count {}",
                    weights.len()
                );
            }
            ratio += 0.01;
        }
    }

    #[test]
    #[should_panic(expected = "exceeding SF_LANCZOS2_MAX_TAPS")]
    fn lanczos2_taps_at_panics_rather_than_truncates() {
        let _ = lanczos2_taps_at(0.0, 4.0);
    }

    #[test]
    fn build_axis_taps_produces_one_tap_per_vertex() {
        let taps = build_axis_taps(33, 0, 0, crate::sf_frame::SF_SAMPLES_PER_FO4_INTERVAL);
        assert_eq!(taps.taps.len(), 33);
        for tap in &taps.taps {
            let sum: f32 = tap.weights[..tap.count as usize].iter().sum();
            assert!((sum - 1.0).abs() < 1e-6);
        }
    }
}
