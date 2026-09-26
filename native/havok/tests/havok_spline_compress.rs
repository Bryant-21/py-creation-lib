use havok_native::animation::spline::{
    CompressedSplineBlob, RotationQuantization, ScalarQuantization, SplineCompressionParams,
    SplineFrame, SplineTransform, compress_spline, compress_spline_with_params, decompress_spline,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn assert_close_f32(a: f32, b: f32, tol: f32, label: &str) {
    assert!(
        (a - b).abs() <= tol,
        "{label}: got {a}, expected {b} (tol={tol})"
    );
}

fn assert_close_vec3(a: &[f32; 3], b: &[f32; 3], tol: f32, label: &str) {
    for i in 0..3 {
        assert_close_f32(a[i], b[i], tol, &format!("{label}[{i}]"));
    }
}

fn assert_close_quat(a: &[f32; 4], b: &[f32; 4], tol: f32, label: &str) {
    // Allow for quaternion sign-flip (q and -q represent the same rotation)
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let sign = if dot < 0.0 { -1.0f32 } else { 1.0f32 };
    for i in 0..4 {
        assert_close_f32(a[i], sign * b[i], tol, &format!("{label}[{i}]"));
    }
}

/// Build synthetic keyframes: `num_tracks` tracks × `num_frames` frames.
/// Track i gets: translation=(i*0.1, i*0.2, i*0.3), rotation=identity, scale=(1,1,1)
fn make_synthetic_frames(num_tracks: usize, num_frames: usize) -> Vec<SplineFrame> {
    (0..num_frames)
        .map(|_| SplineFrame {
            transforms: (0..num_tracks)
                .map(|i| SplineTransform {
                    translation: [i as f32 * 0.1, i as f32 * 0.2, i as f32 * 0.3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                })
                .collect(),
        })
        .collect()
}

/// Build synthetic frames with varying translation over time (tests dynamic tracks).
fn make_dynamic_frames(num_tracks: usize, num_frames: usize) -> Vec<SplineFrame> {
    (0..num_frames)
        .map(|f| SplineFrame {
            transforms: (0..num_tracks)
                .map(|i| SplineTransform {
                    translation: [
                        i as f32 * 0.1 + f as f32 * 0.05,
                        i as f32 * 0.2,
                        i as f32 * 0.3,
                    ],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                })
                .collect(),
        })
        .collect()
}

fn decompress(blob: &CompressedSplineBlob) -> Vec<SplineFrame> {
    decompress_spline(
        &blob.data,
        blob.num_tracks,
        blob.num_floats,
        blob.num_frames,
        blob.max_frames_per_block,
        blob.num_blocks,
        &blob.block_offsets,
        &blob.float_block_offsets,
        blob.mask_and_quant_size,
        blob.block_duration,
        blob.block_inverse_duration,
        blob.frame_duration,
    )
    .expect("decompress_spline")
}

#[test]
fn compress_then_decompress_recovers_keyframes() {
    let default = SplineCompressionParams::default();
    let polar32 = SplineCompressionParams {
        rotation_type: RotationQuantization::Polar32,
        ..SplineCompressionParams::default()
    };
    let bits8_scale = SplineCompressionParams {
        scale_type: ScalarQuantization::Bits8,
        ..SplineCompressionParams::default()
    };
    // (label, frames, params, rotation tolerance)
    let cases = [
        ("static 2x2", make_synthetic_frames(2, 2), &default, 1e-2),
        ("dynamic 1x5", make_dynamic_frames(1, 5), &default, 1e-2),
        ("dynamic 2x30", make_dynamic_frames(2, 30), &default, 1e-2),
        ("polar32 2x4", make_synthetic_frames(2, 4), &polar32, 5e-2),
        (
            "bits8 scale 1x3",
            make_synthetic_frames(1, 3),
            &bits8_scale,
            1e-2,
        ),
    ];
    for (label, frames, params, rot_tol) in cases {
        let duration = (frames.len() - 1) as f32 / 30.0;
        let blob = compress_spline_with_params(&frames, &[], duration, 30.0, params)
            .unwrap_or_else(|e| panic!("{label}: compress failed: {e:?}"));
        assert_eq!(blob.num_frames as usize, frames.len(), "{label}");
        let decompressed = decompress(&blob);
        assert_eq!(decompressed.len(), frames.len(), "{label}: frame count");
        for (fi, frame) in decompressed.iter().enumerate() {
            assert_eq!(
                frame.transforms.len(),
                frames[fi].transforms.len(),
                "{label}"
            );
            for (ti, t) in frame.transforms.iter().enumerate() {
                let e = &frames[fi].transforms[ti];
                let at = format!("{label} frame {fi} track {ti}");
                assert_close_vec3(
                    &t.translation,
                    &e.translation,
                    1e-3,
                    &format!("{at} translation"),
                );
                assert_close_quat(&t.rotation, &e.rotation, rot_tol, &format!("{at} rotation"));
                assert_close_vec3(&t.scale, &e.scale, 1e-3, &format!("{at} scale"));
            }
        }
    }
}

#[test]
fn compressed_blob_metadata_and_float_tracks() {
    compress_spline(&make_synthetic_frames(1, 1), 1.0, 30.0).expect("single-frame compress");

    let blob = compress_spline(&make_synthetic_frames(3, 10), 9.0 / 30.0, 30.0)
        .expect("compress_spline metadata");
    assert_eq!(blob.num_tracks, 3);
    assert_eq!(blob.num_floats, 0);
    assert_eq!(blob.num_frames, 10);
    assert!(blob.num_blocks >= 1);
    assert!(blob.frame_duration > 0.0);
    assert!(blob.block_duration > 0.0);
    assert!(blob.block_inverse_duration > 0.0);
    assert_eq!(blob.block_offsets.len() as u32, blob.num_blocks);
    assert_eq!(blob.mask_and_quant_size, 4 * 3);
    assert!(!blob.data.is_empty());

    let float_tracks: Vec<Vec<f32>> = vec![vec![0.1, 0.2, 0.3, 0.4], vec![1.0, 1.0, 1.0, 1.0]];
    let blob = compress_spline_with_params(
        &make_synthetic_frames(1, 4),
        &float_tracks,
        3.0 / 30.0,
        30.0,
        &SplineCompressionParams::default(),
    )
    .expect("compress_spline_with_params float tracks");
    assert_eq!(blob.num_floats, 2);
    assert_eq!(blob.float_block_offsets.len() as u32, blob.num_blocks);
    // align_up(4 * 1 track + 2 float masks, 4)
    assert_eq!(blob.mask_and_quant_size, 8);
}

#[test]
fn compute_rot_mask_handles_hemisphere_flips() {
    use havok_native::animation::spline::compute_rot_mask;
    use std::f32::consts::FRAC_1_SQRT_2;
    // 4 samples that alternate hemisphere but represent the same rotation.
    // (Use exact unit quat — literal 0.7071 leaves enough rounding to fail
    // the algorithm's 1e-3 angular tolerance even with perfect alignment.)
    let q = [FRAC_1_SQRT_2, 0.0, 0.0, FRAC_1_SQRT_2];
    let nq = [-q[0], -q[1], -q[2], -q[3]];
    let samples = [q, nq, q, nq];
    let mask = compute_rot_mask(&samples, 1e-3);
    // After hemisphere alignment, all four samples represent the same rotation
    // → static, non-identity → 0x0F.
    assert_eq!(
        mask, 0x0F,
        "alternating hemisphere samples should classify as static, got 0x{mask:02X}"
    );
}
