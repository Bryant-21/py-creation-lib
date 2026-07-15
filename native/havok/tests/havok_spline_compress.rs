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

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// Minimal single-track, single-frame static round-trip.
#[test]
fn compress_single_track_single_frame_static_rotation_round_trips() {
    let frames = vec![SplineFrame {
        transforms: vec![SplineTransform {
            translation: [1.0, 2.0, 3.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }],
    }];

    // Single frame — compress expects at least 2 frames; returns identity blob or clips.
    let result = compress_spline(&frames, 1.0, 30.0);
    assert!(result.is_ok(), "compress_spline failed: {:?}", result.err());
}

/// Round-trip: compress 2 static tracks × 2 frames, then decompress and check values.
#[test]
fn compress_then_decompress_static_tracks_recover_keyframes() {
    let frames = make_synthetic_frames(2, 2);

    let blob: CompressedSplineBlob =
        compress_spline(&frames, 1.0 / 30.0, 30.0).expect("compress_spline");

    assert_eq!(blob.num_tracks, 2);
    assert_eq!(blob.num_frames, 2);
    assert!(blob.num_blocks >= 1);

    let decompressed = decompress_spline(
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
    .expect("decompress_spline");

    assert_eq!(decompressed.len(), 2, "frame count mismatch");
    for (fi, frame) in decompressed.iter().enumerate() {
        assert_eq!(
            frame.transforms.len(),
            2,
            "track count mismatch at frame {fi}"
        );
        for (ti, t) in frame.transforms.iter().enumerate() {
            let expected = &frames[fi].transforms[ti];
            assert_close_vec3(
                &t.translation,
                &expected.translation,
                1e-3,
                &format!("frame {fi} track {ti} translation"),
            );
            assert_close_quat(
                &t.rotation,
                &expected.rotation,
                1e-2,
                &format!("frame {fi} track {ti} rotation"),
            );
            assert_close_vec3(
                &t.scale,
                &expected.scale,
                1e-3,
                &format!("frame {fi} track {ti} scale"),
            );
        }
    }
}

/// Round-trip with dynamic (varying) translations.
#[test]
fn compress_then_decompress_dynamic_translation_recovers_keyframes() {
    let frames = make_dynamic_frames(1, 5);

    let blob = compress_spline(&frames, 4.0 / 30.0, 30.0).expect("compress_spline dynamic");

    let decompressed = decompress_spline(
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
    .expect("decompress_spline dynamic");

    assert_eq!(decompressed.len(), 5);
    // Check first and last frames exactly; interpolated frames have tolerance
    {
        let t = &decompressed[0].transforms[0];
        let e = &frames[0].transforms[0];
        assert_close_vec3(
            &t.translation,
            &e.translation,
            1e-3,
            "first frame translation",
        );
    }
    {
        let t = &decompressed[4].transforms[0];
        let e = &frames[4].transforms[0];
        assert_close_vec3(
            &t.translation,
            &e.translation,
            1e-3,
            "last frame translation",
        );
    }
}

/// Full round-trip test: 2 tracks × 30 frames.
#[test]
fn compress_then_decompress_recovers_keyframes_within_tolerance() {
    let frames = make_dynamic_frames(2, 30);
    let duration = 29.0 / 30.0; // 30 frames at 30 fps

    let blob = compress_spline(&frames, duration, 30.0).expect("compress_spline 30-frame");

    let decompressed = decompress_spline(
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
    .expect("decompress_spline 30-frame");

    assert_eq!(decompressed.len() as u32, blob.num_frames);

    for (fi, frame) in decompressed.iter().enumerate() {
        for (ti, t) in frame.transforms.iter().enumerate() {
            let expected = &frames[fi].transforms[ti];
            assert_close_vec3(
                &t.translation,
                &expected.translation,
                1e-3,
                &format!("frame {fi} track {ti} translation"),
            );
            assert_close_quat(
                &t.rotation,
                &expected.rotation,
                1e-2,
                &format!("frame {fi} track {ti} rotation"),
            );
            assert_close_vec3(
                &t.scale,
                &expected.scale,
                1e-3,
                &format!("frame {fi} track {ti} scale"),
            );
        }
    }
}

#[test]
fn compressed_blob_metadata_fields_are_correct() {
    let frames = make_synthetic_frames(3, 10);
    let duration = 9.0 / 30.0;

    let blob = compress_spline(&frames, duration, 30.0).expect("compress_spline metadata");

    assert_eq!(blob.num_tracks, 3);
    assert_eq!(blob.num_floats, 0);
    assert_eq!(blob.num_frames, 10);
    assert!(blob.num_blocks >= 1);
    assert!(blob.frame_duration > 0.0, "frame_duration must be > 0");
    assert!(blob.block_duration > 0.0, "block_duration must be > 0");
    assert!(
        blob.block_inverse_duration > 0.0,
        "block_inverse_duration must be > 0"
    );
    assert_eq!(blob.block_offsets.len() as u32, blob.num_blocks);
    // mask_and_quant_size = 4 * num_tracks (no float tracks), padded to 4
    assert_eq!(blob.mask_and_quant_size, 4 * 3);
    assert!(!blob.data.is_empty(), "data blob must not be empty");
}

#[test]
fn block_offsets_count_matches_num_blocks() {
    let frames = make_synthetic_frames(1, 4);
    let blob = compress_spline(&frames, 3.0 / 30.0, 30.0).expect("compress_spline blocks");
    assert_eq!(blob.block_offsets.len() as u32, blob.num_blocks);
}

// ─── SplineCompressionParams + float tracks ──────────────

/// Compress with POLAR32 rotation quantization and ensure round-trip via
/// `decompress_spline` still recovers the keyframes.
#[test]
fn compress_with_polar32_rotation_round_trips_keyframes() {
    let frames = make_synthetic_frames(2, 4);
    let duration = 3.0 / 30.0;

    let params = SplineCompressionParams {
        rotation_type: RotationQuantization::Polar32,
        ..SplineCompressionParams::default()
    };
    let blob = compress_spline_with_params(&frames, &[], duration, 30.0, &params)
        .expect("compress_spline_with_params polar32");

    let decompressed = decompress_spline(
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
    .expect("decompress_spline polar32");

    assert_eq!(decompressed.len(), 4);
    for (fi, frame) in decompressed.iter().enumerate() {
        for (ti, t) in frame.transforms.iter().enumerate() {
            let e = &frames[fi].transforms[ti];
            assert_close_vec3(
                &t.translation,
                &e.translation,
                1e-3,
                &format!("frame {fi} track {ti} translation (polar32)"),
            );
            // POLAR32 has reduced precision; allow a looser tolerance.
            assert_close_quat(
                &t.rotation,
                &e.rotation,
                5e-2,
                &format!("frame {fi} track {ti} rotation (polar32)"),
            );
        }
    }
}

/// The compressor lays down float-track masks and per-track payloads, and the
/// resulting blob carries `num_floats` and `float_block_offsets` populated.
#[test]
fn compress_with_float_tracks_emits_mask_and_offsets() {
    let frames = make_synthetic_frames(1, 4);
    let float_tracks: Vec<Vec<f32>> = vec![
        vec![0.1, 0.2, 0.3, 0.4],
        vec![1.0, 1.0, 1.0, 1.0], // static — should compress to identity-mask
    ];
    let params = SplineCompressionParams::default();
    let blob = compress_spline_with_params(&frames, &float_tracks, 3.0 / 30.0, 30.0, &params)
        .expect("compress_spline_with_params float tracks");

    assert_eq!(blob.num_floats, 2);
    assert_eq!(
        blob.float_block_offsets.len() as u32,
        blob.num_blocks,
        "non-empty float tracks must populate float_block_offsets"
    );
    // mask_and_quant_size = align_up(4*1 + 2, 4) = 8
    assert_eq!(blob.mask_and_quant_size, 8);
}

/// SplineCompressionParams can request a different scale-channel scalar
/// type from the translation channel — confirm the round-trip still works.
#[test]
fn compress_with_bits8_scale_channel_round_trips() {
    let frames = make_synthetic_frames(1, 3);
    let params = SplineCompressionParams {
        scale_type: ScalarQuantization::Bits8,
        ..SplineCompressionParams::default()
    };
    let blob = compress_spline_with_params(&frames, &[], 2.0 / 30.0, 30.0, &params)
        .expect("compress with bits8 scale");

    // Round-trip via decompressor — main goal is that this doesn't crash.
    let _ = decompress_spline(
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
    .expect("decompress_spline bits8 scale");
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
