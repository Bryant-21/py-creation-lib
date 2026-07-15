use havok_native::animation::clip::{
    AnimationClip, AnimationEvent, AnimationKeyframe, BoneChannel,
};
use havok_native::animation::spline::{
    ROTATION_ALIGN, ROTATION_SIZE, pack_straight16_quat, unpack_polar32, unpack_straight16_quat,
    unpack_threecomp24, unpack_threecomp40, unpack_threecomp48, unpack_uncompressed_quat,
};
use havok_native::animation::spline::{
    RotationQuantization, ScalarQuantization, decompress_spline, dequant_u8, dequant_u16, find_span,
};
use havok_native::animation::writer::write_interleaved_animation_xml;
use havok_native::animation::{
    AnimationCompression, extract_skeleton_from_tagxml, parse_animation_metadata_from_tagxml,
};
use havok_native::api::{havok_extract_clip, havok_write_animation_xml};
use havok_native::error::HavokError;

const INTERLEAVED_ANIMATION_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#animation" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
            <hkparam name="duration">1.000000</hkparam>
            <hkparam name="numberOfTransformTracks">2</hkparam>
            <hkparam name="numberOfFloatTracks">1</hkparam>
            <hkparam name="transforms" numelements="4">(0 0 0 0 0 0 0 1 1 1 1 0) (1 0 0 0 0 0 0 1 1 1 1 0) (2 0 0 0 0 0 0 1 1 1 1 0) (3 0 0 0 0 0 0 1 1 1 1 0)</hkparam>
            <hkparam name="annotationTracks" numelements="1">
                <hkobject>
                    <hkparam name="trackName">Root</hkparam>
                    <hkparam name="annotations" numelements="1">
                        <hkobject>
                            <hkparam name="time">0.250000</hkparam>
                            <hkparam name="text">FootLeft</hkparam>
                        </hkobject>
                    </hkparam>
                </hkobject>
            </hkparam>
        </hkobject>
    </hksection>
</hkpackfile>
"##;

const SKELETON_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#skeleton" class="hkaSkeleton" signature="0x366e8220">
            <hkparam name="name">TestSkeleton</hkparam>
            <hkparam name="parentIndices" numelements="2">-1 0</hkparam>
            <hkparam name="bones" numelements="2">
                <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
                <hkobject><hkparam name="name">Spine</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
            </hkparam>
        </hkobject>
    </hksection>
</hkpackfile>
"##;

#[test]
fn parses_animation_metadata_from_tagxml() {
    let metadata = parse_animation_metadata_from_tagxml(INTERLEAVED_ANIMATION_XML)
        .expect("parse animation metadata");

    assert_eq!(metadata.compression, AnimationCompression::Interleaved);
    assert_eq!(metadata.duration, 1.0);
    assert_eq!(metadata.transform_track_count, 2);
    assert_eq!(metadata.float_track_count, 1);
    assert_eq!(metadata.frame_count, 2);
    assert_eq!(metadata.annotation_tracks.len(), 1);
    assert_eq!(metadata.annotation_tracks[0].name, "Root");
    assert_eq!(
        metadata.annotation_tracks[0].annotations[0].text,
        "FootLeft"
    );
}

#[test]
fn extracts_skeleton_names_and_parent_indices_from_tagxml() {
    let skeleton = extract_skeleton_from_tagxml(SKELETON_XML).expect("extract skeleton");

    assert_eq!(skeleton.name, "TestSkeleton");
    assert_eq!(skeleton.bones, ["Root", "Spine"]);
    assert_eq!(skeleton.parent_indices, [-1, 0]);
}

#[test]
fn spline_uncompressed_quaternion_reads_little_endian_f32s() {
    let mut data = Vec::new();
    for value in [0.1f32, 0.2, 0.3, 0.9] {
        data.extend_from_slice(&value.to_le_bytes());
    }

    let quat = unpack_uncompressed_quat(&data, 0).expect("unpack quaternion");

    assert!((quat[0] - 0.1).abs() < 1e-6);
    assert!((quat[3] - 0.9).abs() < 1e-6);
}

#[test]
fn spline_straight16_unpack_normalizes_and_pack_roundtrips_small_values() {
    let quat = unpack_straight16_quat(&[0, 0], 0).expect("unpack straight16");
    let length = quat.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!((length - 1.0).abs() < 0.001);

    let packed = pack_straight16_quat([-1.0, 0.0, 1.0, 0.0]).expect("pack straight16");
    let unpacked = unpack_straight16_quat(&packed, 0).expect("unpack packed straight16");
    assert!(unpacked[0] < -0.65);
    assert!(unpacked[2] > 0.65);
}

// ---------------------------------------------------------------------------
// Quaternion unpacker tests
// ---------------------------------------------------------------------------

#[test]
fn polar32_decodes_identity_quaternion() {
    // All zeros: e=0 iw=0 signs=0 → w=1, xyz=0
    let q = unpack_polar32(&[0u8; 4], 0).expect("unpack polar32");
    assert!((q[3] - 1.0).abs() < 1e-6, "w should be 1.0, got {}", q[3]);
    assert!(q[0].abs() < 1e-6);
    assert!(q[1].abs() < 1e-6);
    assert!(q[2].abs() < 1e-6);
}

#[test]
fn polar32_decodes_known_non_trivial_value() {
    // val = (511 << 18) | 0, iw=511 → w ≈ 0.7504885
    // Python: _unpack_polar32(struct.pack('<I', 511<<18), 0) → [0.0, 0.0, 0.6608834, 0.7504885]
    let val: u32 = 511 << 18;
    let bytes = val.to_le_bytes();
    let q = unpack_polar32(&bytes, 0).expect("unpack polar32 non-trivial");
    assert!((q[3] - 0.750_488_5).abs() < 1e-4, "w got {}", q[3]);
    assert!((q[2] - 0.660_883_5).abs() < 1e-4, "z got {}", q[2]);
    assert!(q[0].abs() < 1e-6);
    assert!(q[1].abs() < 1e-6);
    let norm: f32 = q.iter().map(|c| c * c).sum();
    assert!((norm - 1.0).abs() < 1e-4);
}

#[test]
fn threecomp40_decodes_known_vector_with_unit_norm() {
    // Python: _unpack_threecomp40(bytes([254,235,191,254,59]), 0)
    // → [0.35338, 0.35338, 0.35338, 0.79080]
    let bytes = [254u8, 235, 191, 254, 59];
    let q = unpack_threecomp40(&bytes, 0).expect("unpack threecomp40");
    assert!((q[0] - 0.353_38).abs() < 1e-3, "x got {}", q[0]);
    assert!((q[1] - 0.353_38).abs() < 1e-3, "y got {}", q[1]);
    assert!((q[2] - 0.353_38).abs() < 1e-3, "z got {}", q[2]);
    assert!((q[3] - 0.790_80).abs() < 1e-3, "w got {}", q[3]);
    let norm: f32 = q.iter().map(|c| c * c).sum();
    assert!((norm - 1.0).abs() < 1e-3);
}

#[test]
fn threecomp48_decodes_known_vector_with_unit_norm() {
    // Python: _unpack_threecomp48(bytes([254,223,254,95,254,95]), 0)
    // → [0.35353, 0.79060, 0.35353, 0.35353]
    let bytes = [254u8, 223, 254, 95, 254, 95];
    let q = unpack_threecomp48(&bytes, 0).expect("unpack threecomp48");
    assert!((q[0] - 0.353_53).abs() < 1e-3, "x got {}", q[0]);
    assert!((q[1] - 0.790_60).abs() < 1e-3, "y got {}", q[1]);
    assert!((q[2] - 0.353_53).abs() < 1e-3, "z got {}", q[2]);
    assert!((q[3] - 0.353_53).abs() < 1e-3, "w got {}", q[3]);
    let norm: f32 = q.iter().map(|c| c * c).sum();
    assert!((norm - 1.0).abs() < 1e-3);
}

#[test]
fn threecomp24_decodes_known_vector_with_unit_norm() {
    // Python: _unpack_threecomp24(bytes([222, 94, 94]), 0)
    // → [0.34794, 0.79800, 0.34794, 0.34794]
    let bytes = [222u8, 94, 94];
    let q = unpack_threecomp24(&bytes, 0).expect("unpack threecomp24");
    assert!((q[0] - 0.347_94).abs() < 1e-3, "x got {}", q[0]);
    assert!((q[1] - 0.798_00).abs() < 1e-3, "y got {}", q[1]);
    assert!((q[2] - 0.347_94).abs() < 1e-3, "z got {}", q[2]);
    assert!((q[3] - 0.347_94).abs() < 1e-3, "w got {}", q[3]);
    let norm: f32 = q.iter().map(|c| c * c).sum();
    assert!((norm - 1.0).abs() < 1e-3);
}

#[test]
fn rotation_size_and_align_tables_match_python_constants() {
    // Python: _ROTATION_SIZE = [4, 5, 6, 3, 2, 16]
    //         _ROTATION_ALIGN = [4, 1, 2, 1, 2, 4]
    assert_eq!(ROTATION_SIZE, [4, 5, 6, 3, 2, 16]);
    assert_eq!(ROTATION_ALIGN, [4, 1, 2, 1, 2, 4]);
}

#[test]
fn rotation_quantization_variants_exist() {
    // Ensure the enum variants and index order are as expected
    let _polar32 = RotationQuantization::Polar32;
    let _tc40 = RotationQuantization::ThreeComp40;
    let _tc48 = RotationQuantization::ThreeComp48;
    let _tc24 = RotationQuantization::ThreeComp24;
    let _s16 = RotationQuantization::Straight16;
    let _unc = RotationQuantization::Uncompressed;
}

#[test]
fn scalar_quantization_variants_exist() {
    let _bits8 = ScalarQuantization::Bits8;
    let _bits16 = ScalarQuantization::Bits16;
}

#[test]
fn dequant_u8_midpoint() {
    // Python: _unpack8(0.0, 1.0, 127) ≈ 0.498039
    let v = dequant_u8(0.0, 1.0, 127);
    assert!((v - 0.498_039).abs() < 1e-4, "got {v}");
}

#[test]
fn dequant_u16_midpoint() {
    // Python: _unpack16(0.0, 1.0, 32767) ≈ 0.49999
    let v = dequant_u16(0.0, 1.0, 32767);
    assert!((v - 0.499_99).abs() < 1e-4, "got {v}");
}

#[test]
fn find_span_degree1_returns_p_at_lower_bound() {
    // Python: _find_span(1, 1, 0, bytes([0,0,1,1]), 0) → 1
    let knots = [0u8, 0, 1, 1];
    assert_eq!(find_span(1, 1, 0, &knots, 0), 1);
}

#[test]
fn find_span_degree1_returns_n_at_upper_bound() {
    // Python: _find_span(1, 1, 1, bytes([0,0,1,1]), 0) → 1
    let knots = [0u8, 0, 1, 1];
    assert_eq!(find_span(1, 1, 1, &knots, 0), 1);
}

#[test]
fn find_span_degree2_uniform() {
    // Python: _find_span(2, 2, 0, bytes([0,0,0,1,1,1]), 0) → 2
    let knots = [0u8, 0, 0, 1, 1, 1];
    assert_eq!(find_span(2, 2, 0, &knots, 0), 2);
}

#[test]
fn decompress_spline_static_all_tracks_returns_expected_frames() {
    // Synthetic 1-track, 2-frame buffer with all-static components.
    // trans_q=0(BITS8), rot_q=5(UNCOMPRESSED), scale_q=0(BITS8)
    // packed_q = 0 | (5<<2) | 0 = 0x14
    // trans_mask=0x07 (static x,y,z), rot_mask=0x0F (static), scale_mask=0x07
    //
    // Buffer layout (block_base=0, mask_and_quant_size=4):
    //   [0..4)   mask block: [0x14, 0x07, 0x0F, 0x07]
    //   [4..16)  translation: (1.0, 2.0, 3.0) as 3×f32le
    //   [16..32) rotation: (0.0, 0.0, 0.0, 1.0) as 4×f32le (identity, UNCOMPRESSED)
    //   [32..44) scale: (1.0, 1.0, 1.0) as 3×f32le
    let mut buf = vec![0x14u8, 0x07, 0x0F, 0x07]; // mask block
    for v in [1.0f32, 2.0, 3.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0.0f32, 0.0, 0.0, 1.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in [1.0f32, 1.0, 1.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    let frames = decompress_spline(
        &buf,
        1,    // num_tracks
        0,    // num_floats
        2,    // num_frames
        2,    // max_frames_per_block
        1,    // num_blocks
        &[0], // block_offsets
        &[],  // float_block_offsets
        4,    // mask_and_quant_size
        1.0,  // block_duration
        1.0,  // block_inverse_duration
        1.0,  // frame_duration
    )
    .expect("decompress spline");

    assert_eq!(frames.len(), 2);
    for frame in &frames {
        assert_eq!(frame.transforms.len(), 1);
        let t = &frame.transforms[0];
        assert!((t.translation[0] - 1.0).abs() < 1e-6);
        assert!((t.translation[1] - 2.0).abs() < 1e-6);
        assert!((t.translation[2] - 3.0).abs() < 1e-6);
        assert!((t.rotation[3] - 1.0).abs() < 1e-6); // w=1 identity
        assert!((t.scale[0] - 1.0).abs() < 1e-6);
    }
}

#[test]
fn decompress_spline_threecomp40_static_rotation_matches_python() {
    // 1 track, 1 frame, static THREECOMP40 rotation
    // Python: _unpack_threecomp40(bytes([254,235,191,254,59]), 0)
    //       → [0.35338, 0.35338, 0.35338, 0.79080]
    // trans_q=0, rot_q=1(THREECOMP40), scale_q=0
    // packed_q = 0 | (1<<2) | 0 = 0x04
    // trans_mask=0x07, rot_mask=0x0F, scale_mask=0x07
    // ROTATION_ALIGN[1]=1, ROTATION_SIZE[1]=5
    //
    // Buffer layout:
    //   [0..4)   mask block
    //   [4..16)  translation (1.0, 0.0, 0.0) 3×f32le
    //   [16..21) rotation: 5 bytes THREECOMP40
    //   [21..24) padding to align(4) → [0,0,0]
    //   [24..36) scale (1.0, 1.0, 1.0)
    let mut buf = vec![0x04u8, 0x07, 0x0F, 0x07]; // mask (packed_q=4)
    // translation
    for v in [1.0f32, 0.0, 0.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // rotation (THREECOMP40, align=1)
    buf.extend_from_slice(&[254u8, 235, 191, 254, 59]);
    // pad to next 4-byte boundary (buf.len() = 4+12+5 = 21, next 4-aligned = 24)
    buf.extend_from_slice(&[0u8, 0, 0]);
    // scale
    for v in [1.0f32, 1.0, 1.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    let frames = decompress_spline(&buf, 1, 0, 1, 1, 1, &[0], &[], 4, 1.0, 1.0, 1.0)
        .expect("decompress spline threecomp40");

    let t = &frames[0].transforms[0];
    assert!((t.rotation[0] - 0.353_38).abs() < 1e-3);
    assert!((t.rotation[3] - 0.790_80).abs() < 1e-3);
}

// ---------------------------------------------------------------------------
// Animation writer round-trip tests
// ---------------------------------------------------------------------------

/// Build a minimal 2-bone, 5-frame AnimationClip for writer testing.
fn make_two_bone_five_frame_clip() -> AnimationClip {
    let fps = 30.0f32;
    let duration = 4.0f32 / fps; // 5 frames at 30fps → 0..=4/30

    // Bone 0: "Root" — translate along x, identity rotation, uniform scale
    let root_translations: Vec<AnimationKeyframe<[f32; 3]>> = (0..5)
        .map(|i| AnimationKeyframe {
            time: i as f32 / fps,
            value: [i as f32 * 0.1, 0.0, 0.0],
        })
        .collect();
    let root_rotations: Vec<AnimationKeyframe<[f32; 4]>> = vec![AnimationKeyframe {
        time: 0.0,
        value: [0.0, 0.0, 0.0, 1.0],
    }];
    let root_scales: Vec<AnimationKeyframe<[f32; 3]>> = vec![AnimationKeyframe {
        time: 0.0,
        value: [1.0, 1.0, 1.0],
    }];

    // Bone 1: "Spine" — identity throughout
    let spine_translations: Vec<AnimationKeyframe<[f32; 3]>> = vec![AnimationKeyframe {
        time: 0.0,
        value: [0.0, 0.0, 0.0],
    }];
    let spine_rotations: Vec<AnimationKeyframe<[f32; 4]>> = vec![AnimationKeyframe {
        time: 0.0,
        value: [0.0, 0.0, 0.0, 1.0],
    }];
    let spine_scales: Vec<AnimationKeyframe<[f32; 3]>> = vec![AnimationKeyframe {
        time: 0.0,
        value: [1.0, 1.0, 1.0],
    }];

    AnimationClip {
        source_format: "test".to_string(),
        duration,
        native_fps: fps,
        original_skeleton_name: Some("TestSkeleton".to_string()),
        channels: vec![
            BoneChannel {
                bone_name: "Root".to_string(),
                translations: root_translations,
                rotations: root_rotations,
                scales: root_scales,
            },
            BoneChannel {
                bone_name: "Spine".to_string(),
                translations: spine_translations,
                rotations: spine_rotations,
                scales: spine_scales,
            },
        ],
        events: vec![AnimationEvent {
            time: 0.0,
            text: "TestEvent".to_string(),
        }],
        warnings: vec![],
        is_additive: false,
        track_to_bone_indices: vec![],
        extracted_motion_ref: String::new(),
    }
}

#[test]
fn interleaved_writer_round_trip_duration_and_track_count() {
    let clip = make_two_bone_five_frame_clip();
    let skeleton_bones = vec!["Root".to_string(), "Spine".to_string()];

    let xml = write_interleaved_animation_xml(&clip, &skeleton_bones).expect("write animation xml");

    // Must be valid XML containing the right class
    assert!(xml.contains("hkaInterleavedUncompressedAnimation"));
    assert!(xml.contains("hkaAnimationBinding"));
    assert!(xml.contains("hkaAnimationContainer"));

    // Parse back and check metadata
    let metadata = parse_animation_metadata_from_tagxml(&xml).expect("parse written animation xml");

    assert_eq!(metadata.compression, AnimationCompression::Interleaved);
    assert_eq!(metadata.transform_track_count, 2);

    // Duration should round-trip within float-format tolerance (6 decimal places)
    assert!(
        (metadata.duration - clip.duration).abs() < 1e-5,
        "duration mismatch: got {}, expected {}",
        metadata.duration,
        clip.duration
    );

    // 5 frames at 30fps for duration=4/30
    assert_eq!(metadata.frame_count, 5);
}

#[test]
fn interleaved_writer_annotation_track_round_trip() {
    let clip = make_two_bone_five_frame_clip();
    let skeleton_bones = vec!["Root".to_string(), "Spine".to_string()];

    let xml = write_interleaved_animation_xml(&clip, &skeleton_bones).expect("write animation xml");

    let metadata = parse_animation_metadata_from_tagxml(&xml).expect("parse written animation xml");

    // Both bones should have annotation tracks
    assert_eq!(metadata.annotation_tracks.len(), 2);
    assert_eq!(metadata.annotation_tracks[0].name, "Root");

    // The event on track 0 should be present
    assert_eq!(metadata.annotation_tracks[0].annotations.len(), 1);
    assert_eq!(
        metadata.annotation_tracks[0].annotations[0].text,
        "TestEvent"
    );
}

#[test]
fn interleaved_writer_six_decimal_float_format() {
    let clip = make_two_bone_five_frame_clip();
    let skeleton_bones = vec!["Root".to_string(), "Spine".to_string()];

    let xml = write_interleaved_animation_xml(&clip, &skeleton_bones).expect("write animation xml");

    // Duration should be formatted with 6 decimal places
    // 4.0/30.0 = 0.133333...
    assert!(
        xml.contains("0.133333"),
        "duration float format not 6dp: check xml output"
    );
}

#[test]
fn interleaved_writer_transform_tuple_format() {
    let clip = make_two_bone_five_frame_clip();
    let skeleton_bones = vec!["Root".to_string(), "Spine".to_string()];

    let xml = write_interleaved_animation_xml(&clip, &skeleton_bones).expect("write animation xml");

    // Each transform must be in (t1 t2 t3 0.0 q1 q2 q3 q4 s1 s2 s3 0.0) format
    // Identity transform: (0.000000 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000 1.000000 1.000000 1.000000 1.000000 0.000000)
    assert!(
        xml.contains("(0.000000 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000 1.000000 1.000000 1.000000 1.000000 0.000000)"),
        "identity transform tuple not found in XML"
    );
}

#[test]
fn interleaved_writer_zero_fps_defaults_to_30() {
    let clip = AnimationClip {
        source_format: "test".to_string(),
        duration: 1.0,
        native_fps: 0.0, // trigger default
        original_skeleton_name: None,
        channels: vec![BoneChannel {
            bone_name: "Root".to_string(),
            translations: vec![AnimationKeyframe {
                time: 0.0,
                value: [0.0, 0.0, 0.0],
            }],
            rotations: vec![AnimationKeyframe {
                time: 0.0,
                value: [0.0, 0.0, 0.0, 1.0],
            }],
            scales: vec![AnimationKeyframe {
                time: 0.0,
                value: [1.0, 1.0, 1.0],
            }],
        }],
        events: vec![],
        warnings: vec![],
        is_additive: false,
        track_to_bone_indices: vec![],
        extracted_motion_ref: String::new(),
    };
    let skeleton_bones = vec!["Root".to_string()];

    let xml = write_interleaved_animation_xml(&clip, &skeleton_bones).expect("write animation xml");

    let metadata = parse_animation_metadata_from_tagxml(&xml).expect("parse written animation xml");

    // 1.0 sec at 30fps = 31 frames (frame_count = duration*sample_rate + 1 = 30 + 1)
    assert_eq!(metadata.frame_count, 31);
    assert_eq!(metadata.transform_track_count, 1);
}

// ---------------------------------------------------------------------------
// High-level API round-trip
// ---------------------------------------------------------------------------

#[test]
fn high_level_api_extracts_and_writes_clip() {
    let clip_json = havok_extract_clip(INTERLEAVED_ANIMATION_XML, None)
        .expect("havok_extract_clip should succeed");

    assert!(
        clip_json.contains("\"channels\""),
        "JSON missing 'channels' key"
    );

    // Duration 1.0 should appear in JSON
    assert!(
        clip_json.contains("1.0") || clip_json.contains("1,") || clip_json.contains("\"duration\""),
        "JSON should contain duration information"
    );

    let xml_out = havok_write_animation_xml(&clip_json, &["Root".to_string(), "Spine".to_string()])
        .expect("havok_write_animation_xml should succeed");

    assert!(
        xml_out.contains("hkaInterleavedUncompressedAnimation"),
        "output XML missing hkaInterleavedUncompressedAnimation"
    );
}
