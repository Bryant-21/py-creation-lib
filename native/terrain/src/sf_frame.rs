//! Starfield <-> FO4 world-frame conversion: pure f64/i32 math, no I/O.
//!
//! Starfield BTD terrain: 100m cells, 0.78125m samples. FO4 LAND: 4096-unit cells
//! (69.99125 units/m), 32 intervals (33 vertices) per cell.

pub const FO4_UNITS_PER_METER: f64 = 69.99125;
pub const FO4_CELL_UNITS: f64 = 4096.0;
pub const FO4_LAND_VERTEX_UNITS: f64 = 128.0;
pub const FO4_LAND_INTERVALS: usize = 32;
pub const SF_CELL_METERS: f64 = 100.0;
pub const SF_BTD_CELL_SAMPLES: usize = 128;
pub const SF_BTD_SAMPLE_METERS: f64 = 0.78125;
pub const SF_BTD_QUADRANT_SAMPLES: usize = 64;
// Vanilla Starfield BTD samples are unshifted. The sample<->units math still
// carries this bias; a wrong value displaces terrain 50m diagonally in-game.
pub const SF_BTD_ORIGIN_BIAS_METERS: f64 = 0.0;
pub const SF_SAMPLES_PER_FO4_INTERVAL: f64 = 2.340864036576;

pub fn meters_to_fo4_units(m: f64) -> f64 {
    m * FO4_UNITS_PER_METER
}

pub fn fo4_units_to_meters(u: f64) -> f64 {
    u / FO4_UNITS_PER_METER
}

/// FO4-unit world position of LAND vertex `v` (0..=32) in cell `cell`.
pub fn fo4_land_vertex_units(cell: i32, v: usize) -> f64 {
    cell as f64 * FO4_CELL_UNITS + v as f64 * FO4_LAND_VERTEX_UNITS
}

fn btd_sample_origin_meters(btd_cell_min: i32) -> f64 {
    btd_cell_min as f64 * SF_CELL_METERS - SF_BTD_ORIGIN_BIAS_METERS
}

/// Fractional global BTD sample index, 0-based from `btd_cell_min`.
pub fn fo4_units_to_btd_sample(units: f64, btd_cell_min: i32) -> f64 {
    (fo4_units_to_meters(units) - btd_sample_origin_meters(btd_cell_min)) / SF_BTD_SAMPLE_METERS
}

pub fn btd_sample_to_fo4_units(sample: f64, btd_cell_min: i32) -> f64 {
    meters_to_fo4_units(btd_sample_origin_meters(btd_cell_min) + sample * SF_BTD_SAMPLE_METERS)
}

/// Floor-division FO4 cell index containing a world position (negatives OK).
pub fn fo4_cell_of_units(units: f32) -> i32 {
    (units as f64 / FO4_CELL_UNITS).floor() as i32
}

pub fn fo4_exterior_block(cell: i32) -> i32 {
    cell.div_euclid(32)
}

pub fn fo4_exterior_sub_block(cell: i32) -> i32 {
    cell.div_euclid(8)
}

/// SF (100m) cell index containing an FO4-unit world position.
pub fn sf_cell_of_fo4_units(units: f64) -> i32 {
    floor_index(fo4_units_to_meters(units) / SF_CELL_METERS)
}

fn floor_index(x: f64) -> i32 {
    x.floor() as i32
}

/// Greatest integer strictly less than `x`: the last index before a half-open
/// upper bound at `x`, also when `x` is exactly an integer.
fn last_index_before(x: f64) -> i32 {
    (x.ceil() - 1.0) as i32
}

/// Inclusive FO4 cell window whose union covers the BTD cell extent
/// `[btd_cell_min, btd_cell_max]` (both inclusive, in SF 100m cells).
pub fn fo4_cell_range(btd_cell_min: i32, btd_cell_max: i32) -> (i32, i32) {
    let start_units = meters_to_fo4_units(btd_cell_min as f64 * SF_CELL_METERS);
    let end_units = meters_to_fo4_units((btd_cell_max + 1) as f64 * SF_CELL_METERS);
    (
        floor_index(start_units / FO4_CELL_UNITS),
        last_index_before(end_units / FO4_CELL_UNITS),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfOverlap {
    pub first: i32,
    pub last: i32,
    pub first_fraction: f64,
}

/// SF (100m) cells overlapping FO4 cell `cell`. `last - first` is always 0
/// or 1 because an FO4 cell (~58.52m) is narrower than an SF cell (100m).
/// `first_fraction` is the fraction of the FO4 cell's span covered by the
/// `first` SF cell (1.0 when `first == last`).
pub fn sf_cells_overlapping_fo4_cell(cell: i32) -> SfOverlap {
    let start_m = fo4_units_to_meters(cell as f64 * FO4_CELL_UNITS);
    let end_m = fo4_units_to_meters((cell + 1) as f64 * FO4_CELL_UNITS);
    let first = floor_index(start_m / SF_CELL_METERS);
    let last = last_index_before(end_m / SF_CELL_METERS);
    let first_cell_end_m = (first + 1) as f64 * SF_CELL_METERS;
    let covered_in_first = (end_m.min(first_cell_end_m) - start_m).max(0.0);
    let first_fraction = covered_in_first / (end_m - start_m);
    SfOverlap {
        first,
        last,
        first_fraction,
    }
}

/// Inclusive FO4 cell window overlapping SF cell `cell`. Span (`last -
/// first + 1`) is always 2 or 3 because an SF cell (100m) is wider than one
/// FO4 cell (~58.52m) but narrower than two.
pub fn fo4_cells_overlapping_sf_cell(cell: i32) -> (i32, i32) {
    let start_units = meters_to_fo4_units(cell as f64 * SF_CELL_METERS);
    let end_units = meters_to_fo4_units((cell + 1) as f64 * SF_CELL_METERS);
    (
        floor_index(start_units / FO4_CELL_UNITS),
        last_index_before(end_units / FO4_CELL_UNITS),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfQuadrantSpan {
    pub sf_cell: i32,
    /// Which half (0 = low, 1 = high) of `sf_cell`'s own 50m span this
    /// fragment falls in.
    pub sf_quadrant_axis: u8,
    pub meters: f64,
}

/// Splits one axis of FO4 cell `cell`'s quadrant `q` (0 = low half, 1 = high
/// half; ~29.26m wide) at SF 100m cell and 50m half-cell boundaries. The
/// quadrant is narrower than half an SF cell, so it crosses at most one boundary
/// and yields at most two fragments. Sorted longest first; on an exact tie the
/// fragment containing the quadrant's centre sorts first.
pub fn sf_quadrant_spans_for_fo4_quadrant(cell: i32, q: usize) -> [Option<SfQuadrantSpan>; 2] {
    let half = FO4_CELL_UNITS / 2.0;
    let start_units = cell as f64 * FO4_CELL_UNITS + q as f64 * half;
    let end_units = start_units + half;
    let start_m = fo4_units_to_meters(start_units);
    let end_m = fo4_units_to_meters(end_units);

    let sf_first = floor_index(start_m / SF_CELL_METERS);
    let sf_last = last_index_before(end_m / SF_CELL_METERS);

    let mut fragments: Vec<SfQuadrantSpan> = Vec::with_capacity(2);
    if sf_first == sf_last {
        let mid_m = sf_first as f64 * SF_CELL_METERS + SF_CELL_METERS / 2.0;
        if start_m < mid_m && end_m > mid_m {
            fragments.push(SfQuadrantSpan {
                sf_cell: sf_first,
                sf_quadrant_axis: 0,
                meters: mid_m - start_m,
            });
            fragments.push(SfQuadrantSpan {
                sf_cell: sf_first,
                sf_quadrant_axis: 1,
                meters: end_m - mid_m,
            });
        } else {
            let axis: u8 = if end_m <= mid_m { 0 } else { 1 };
            fragments.push(SfQuadrantSpan {
                sf_cell: sf_first,
                sf_quadrant_axis: axis,
                meters: end_m - start_m,
            });
        }
    } else {
        let boundary_m = sf_last as f64 * SF_CELL_METERS;
        fragments.push(SfQuadrantSpan {
            sf_cell: sf_first,
            sf_quadrant_axis: 1,
            meters: boundary_m - start_m,
        });
        fragments.push(SfQuadrantSpan {
            sf_cell: sf_last,
            sf_quadrant_axis: 0,
            meters: end_m - boundary_m,
        });
    }

    if fragments.len() == 2 && fragments[1].meters >= fragments[0].meters {
        fragments.swap(0, 1);
    }

    let mut out: [Option<SfQuadrantSpan>; 2] = [None, None];
    for (slot, fragment) in out.iter_mut().zip(fragments) {
        *slot = Some(fragment);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FO4_QUADRANT_WIDTH_METERS: f64 = 29.2608;

    #[test]
    fn sf_samples_per_fo4_interval_matches_derivation() {
        let derived = FO4_LAND_VERTEX_UNITS / (FO4_UNITS_PER_METER * SF_BTD_SAMPLE_METERS);
        assert!(
            (derived - SF_SAMPLES_PER_FO4_INTERVAL).abs() < 1e-9,
            "derived {derived} vs constant {SF_SAMPLES_PER_FO4_INTERVAL}"
        );
    }

    #[test]
    fn sf_cell_meters_in_fo4_units_matches_expected_literal() {
        // 100.0*69.99125/4096.0 is 1 ULP off exact `==` against the decimal
        // literal 1.708770751953125 (verified: 0x1.b571fffffffffp+0 vs
        // 0x1.b572000000000p+0) — 69.99125 itself isn't exactly representable
        // in binary64, so bit-exact `==` against a hand-typed decimal is not
        // achievable; a tight epsilon is the faithful equivalent.
        let computed = SF_CELL_METERS * FO4_UNITS_PER_METER / FO4_CELL_UNITS;
        assert!(
            (computed - 1.708770751953125).abs() < 1e-12,
            "computed {computed}"
        );
    }

    #[test]
    fn sf_btd_origin_bias_is_zero_per_r4_section_7_3() {
        assert_eq!(
            SF_BTD_ORIGIN_BIAS_METERS, 0.0,
            "R4 §7.3: proven unshifted — a nonzero bias here regresses as terrain \
             displaced 50m diagonally in-game"
        );
    }

    #[test]
    fn meters_and_fo4_units_round_trip() {
        for m in [-1234.5, -1.0, 0.0, 0.78125, 100.0, 987654.321] {
            let back = fo4_units_to_meters(meters_to_fo4_units(m));
            assert!((back - m).abs() < 1e-9, "m {m} back {back}");
        }
    }

    #[test]
    fn fo4_land_vertex_units_spans_one_cell() {
        assert_eq!(fo4_land_vertex_units(0, 0), 0.0);
        assert_eq!(fo4_land_vertex_units(0, 32), 4096.0);
        assert_eq!(fo4_land_vertex_units(1, 0), 4096.0);
        assert_eq!(fo4_land_vertex_units(-1, 32), 0.0);
    }

    #[test]
    fn btd_sample_fo4_units_round_trip() {
        for btd_cell_min in [-5, -1, 0, 1, 7] {
            for i in 0..37 {
                let sample = i as f64 * 3.7 - 12.0;
                let units = btd_sample_to_fo4_units(sample, btd_cell_min);
                let back = fo4_units_to_btd_sample(units, btd_cell_min);
                assert!(
                    (back - sample).abs() < 1e-6,
                    "sample {sample} btd_cell_min {btd_cell_min} back {back}"
                );
            }
        }
    }

    #[test]
    fn fo4_cell_of_units_floors_including_negatives() {
        assert_eq!(fo4_cell_of_units(0.0), 0);
        assert_eq!(fo4_cell_of_units(4095.0), 0);
        assert_eq!(fo4_cell_of_units(4096.0), 1);
        assert_eq!(fo4_cell_of_units(-1.0), -1);
        assert_eq!(fo4_cell_of_units(-4096.0), -1);
        assert_eq!(fo4_cell_of_units(-4097.0), -2);
    }

    #[test]
    fn fo4_exterior_block_and_sub_block_use_euclidean_division() {
        assert_eq!(fo4_exterior_block(31), 0);
        assert_eq!(fo4_exterior_block(32), 1);
        assert_eq!(fo4_exterior_block(-1), -1);
        assert_eq!(fo4_exterior_block(-32), -1);
        assert_eq!(fo4_exterior_block(-33), -2);

        assert_eq!(fo4_exterior_sub_block(7), 0);
        assert_eq!(fo4_exterior_sub_block(8), 1);
        assert_eq!(fo4_exterior_sub_block(-1), -1);
        assert_eq!(fo4_exterior_sub_block(-8), -1);
        assert_eq!(fo4_exterior_sub_block(-9), -2);
    }

    #[test]
    fn sf_cell_of_fo4_units_matches_meter_boundaries() {
        assert_eq!(sf_cell_of_fo4_units(meters_to_fo4_units(0.0)), 0);
        assert_eq!(sf_cell_of_fo4_units(meters_to_fo4_units(99.9)), 0);
        assert_eq!(sf_cell_of_fo4_units(meters_to_fo4_units(100.0)), 1);
        assert_eq!(sf_cell_of_fo4_units(meters_to_fo4_units(-0.1)), -1);
    }

    #[test]
    fn fo4_cell_range_matches_expected_windows() {
        assert_eq!(fo4_cell_range(-4, 4), (-7, 8));
        assert_eq!(fo4_cell_range(-7, 6), (-12, 11));
        assert_eq!(fo4_cell_range(-5, 4), (-9, 8));
        assert_eq!(fo4_cell_range(-5, 5), (-9, 10));
    }

    #[test]
    fn sf_cells_overlapping_fo4_cell_spans_at_most_two() {
        for c in -64..64 {
            let overlap = sf_cells_overlapping_fo4_cell(c);
            let span = overlap.last - overlap.first;
            assert!(span == 0 || span == 1, "cell {c} span {span}");
            assert!(overlap.first_fraction > 0.0 && overlap.first_fraction <= 1.0 + 1e-9);
        }
    }

    #[test]
    fn fo4_cells_overlapping_sf_cell_spans_two_or_three() {
        for c in -16..16 {
            let (first, last) = fo4_cells_overlapping_sf_cell(c);
            let span = last - first + 1;
            assert!(span == 2 || span == 3, "cell {c} span {span}");
        }
    }

    #[test]
    fn sf_quadrant_spans_two_some_sum_to_quadrant_width() {
        let mut saw_two_some = false;
        for cell in -32..32 {
            for q in 0..2usize {
                let spans = sf_quadrant_spans_for_fo4_quadrant(cell, q);
                assert!(spans.iter().filter(|s| s.is_some()).count() <= 2);
                if let [Some(a), Some(b)] = spans {
                    saw_two_some = true;
                    assert!(
                        a.meters >= b.meters,
                        "not sorted longest-first: {a:?} {b:?}"
                    );
                    assert!(
                        (a.meters + b.meters - FO4_QUADRANT_WIDTH_METERS).abs() < 1e-6,
                        "cell {cell} q {q} sum {}",
                        a.meters + b.meters
                    );
                }
            }
        }
        assert!(
            saw_two_some,
            "expected at least one two-fragment quadrant in the sampled range"
        );
    }
}
