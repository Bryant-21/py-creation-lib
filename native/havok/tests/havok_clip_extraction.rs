/// Animation Clip Extraction tests.
///
/// Three cases:
///   1. Lossless XML → frame0 keyframes per bone (skeleton-mapped).
///   2. Interleaved XML → multi-frame keyframes (2-bone × 5-frame).
///   3. Spline XML round-trip: synthetic single-block, single-track, static
///      rotation → extract_clip → assert recovered keyframe matches input.
use havok_native::animation::clip::{extract_clip, infer_clip_fps};
use havok_native::animation::parsers::parse_skeleton_xml;
use havok_native::animation::quantized::build_synthetic_static_blob;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Minimal lossless animation with:
///   - 2 bones, 1 frame
///   - staticRotations: bone0=identity, bone1=(0,0,0.707,0.707) (90° Z)
///   - translationTypeAndOffsets: bone0 static (1,0,0), bone1 identity
///   - scaleTypeAndOffsets: both identity
const LOSSLESS_ANIMATION_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#anim" class="hkaLosslessCompressedAnimation" signature="0x00000001">
      <hkparam name="duration">0.033333</hkparam>
      <hkparam name="numberOfTransformTracks">2</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="numberOfFrames">1</hkparam>
      <hkparam name="staticRotations" numelements="1">
        (0.0 0.0 0.7071068 0.7071068)
      </hkparam>
      <hkparam name="rotationTypeAndOffsets" numelements="2">0 1</hkparam>
      <hkparam name="dynamicRotations" numelements="0"></hkparam>
      <hkparam name="staticTranslations" numelements="3">1.0 0.0 0.0</hkparam>
      <hkparam name="translationTypeAndOffsets" numelements="2">4 0</hkparam>
      <hkparam name="dynamicTranslations" numelements="0"></hkparam>
      <hkparam name="staticScales" numelements="0"></hkparam>
      <hkparam name="scaleTypeAndOffsets" numelements="2">0 0</hkparam>
      <hkparam name="dynamicScales" numelements="0"></hkparam>
      <hkparam name="annotationTracks" numelements="0"></hkparam>
    </hkobject>
    <hkobject name="#binding" class="hkaAnimationBinding" signature="0x00000002">
      <hkparam name="originalSkeletonName">TestSkel</hkparam>
      <hkparam name="animation">#anim</hkparam>
      <hkparam name="transformTrackToBoneIndices" numelements="0"></hkparam>
      <hkparam name="blendHint">NORMAL</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

const SKELETON_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#skeleton" class="hkaSkeleton" signature="0x366e8220">
      <hkparam name="name">TestSkel</hkparam>
      <hkparam name="parentIndices" numelements="2">-1 0</hkparam>
      <hkparam name="bones" numelements="2">
        <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
        <hkobject><hkparam name="name">Spine</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="2">
        (0 0 0 0 0 0 0 1 1 1 1 0)
        (0 0 0 0 0 0 0 1 1 1 1 0)
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

/// 2-bone × 5-frame interleaved animation.
/// Compact format: 12-float tuples (tx ty tz 0  qx qy qz qw  sx sy sz 0)
/// Frames: translation x goes 0..4 for bone0, bone1 is identity.
const INTERLEAVED_ANIMATION_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#anim" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
      <hkparam name="duration">0.133333</hkparam>
      <hkparam name="numberOfTransformTracks">2</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="transforms" numelements="10">
        (0.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (0.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (1.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (0.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (2.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (0.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (3.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (0.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (4.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
        (0.0 0.0 0.0 0.0 0.0 0.0 0.0 1.0 1.0 1.0 1.0 0.0)
      </hkparam>
      <hkparam name="annotationTracks" numelements="1">
        <hkobject>
          <hkparam name="trackName">Root</hkparam>
          <hkparam name="annotations" numelements="1">
            <hkobject>
              <hkparam name="time">0.066667</hkparam>
              <hkparam name="text">FootLeft</hkparam>
            </hkobject>
          </hkparam>
        </hkobject>
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

// ---------------------------------------------------------------------------
// Test 1: Lossless XML → frame0 keyframes (skeleton-mapped)
// ---------------------------------------------------------------------------

#[test]
fn lossless_clip_extracts_frame0_keyframes_with_skeleton_bone_names() {
    let skeleton = parse_skeleton_xml(SKELETON_XML).expect("parse skeleton");
    let clip =
        extract_clip(LOSSLESS_ANIMATION_XML, Some(&skeleton)).expect("extract lossless clip");

    assert_eq!(clip.source_format, "hkx");
    assert_eq!(clip.channels.len(), 2);

    // Bone names from skeleton
    assert_eq!(clip.channels[0].bone_name, "Root");
    assert_eq!(clip.channels[1].bone_name, "Spine");

    // Bone 0 rotation: type=0 (identity) → (0,0,0,1)
    assert!(
        !clip.channels[0].rotations.is_empty(),
        "Root should have a rotation keyframe"
    );
    let r0 = &clip.channels[0].rotations[0];
    assert_eq!(r0.time, 0.0);
    assert!(
        (r0.value[3] - 1.0).abs() < 1e-5,
        "Root rotation w should be 1.0"
    );

    // Bone 1 rotation: type=1 (static), offset=0 → staticRotations[0] = (0,0,0.707,0.707)
    // Note: rotationTypeAndOffsets "0 1" means bone0 raw=0 (type=0,off=0), bone1 raw=1 (type=1,off=0)
    assert!(
        !clip.channels[1].rotations.is_empty(),
        "Spine should have a rotation keyframe"
    );
    let r1 = &clip.channels[1].rotations[0];
    assert_eq!(r1.time, 0.0);
    assert!(
        (r1.value[2] - 0.7071068).abs() < 1e-4,
        "Spine rotation z ≈ 0.707"
    );
    assert!(
        (r1.value[3] - 0.7071068).abs() < 1e-4,
        "Spine rotation w ≈ 0.707"
    );

    // translationTypeAndOffsets holds a uint64 per bone; each component's low
    // 16 bits are type(&3)+offset(>>2). bone0 raw=4 → type 0 (identity), so
    // bone0 translation resolves to (0,0,0) regardless of staticTranslations.
    assert!(
        !clip.channels[0].translations.is_empty(),
        "Root should have a translation keyframe"
    );

    // Duration preserved
    assert!((clip.duration - 0.033333).abs() < 1e-4);
}

#[test]
fn lossless_clip_without_skeleton_uses_track_names() {
    let clip =
        extract_clip(LOSSLESS_ANIMATION_XML, None).expect("extract lossless clip without skeleton");

    assert_eq!(clip.channels[0].bone_name, "track_0");
    assert_eq!(clip.channels[1].bone_name, "track_1");
}

// ---------------------------------------------------------------------------
// Test 2: Interleaved XML → multi-frame keyframes
// ---------------------------------------------------------------------------

#[test]
fn interleaved_clip_extracts_multiframe_keyframes() {
    let clip = extract_clip(INTERLEAVED_ANIMATION_XML, None).expect("extract interleaved clip");

    assert_eq!(clip.source_format, "hkx");
    assert_eq!(clip.channels.len(), 2);

    // Bone 0 ("track_0") should have 5 translation keyframes
    let ch0 = &clip.channels[0];
    assert_eq!(
        ch0.translations.len(),
        5,
        "track_0 should have 5 translation keyframes"
    );
    assert_eq!(
        ch0.rotations.len(),
        5,
        "track_0 should have 5 rotation keyframes"
    );

    // Frame 0: translation (0,0,0)
    assert!(
        (ch0.translations[0].value[0]).abs() < 1e-5,
        "frame0 x should be 0"
    );
    // Frame 1: translation (1,0,0)
    assert!(
        (ch0.translations[1].value[0] - 1.0).abs() < 1e-5,
        "frame1 x should be 1.0"
    );
    // Frame 4: translation (4,0,0)
    assert!(
        (ch0.translations[4].value[0] - 4.0).abs() < 1e-5,
        "frame4 x should be 4.0"
    );

    // Timestamps should be evenly spaced
    let dt = ch0.translations[1].time - ch0.translations[0].time;
    assert!(dt > 1e-5, "frame dt should be positive, got {dt}");
    let dt2 = ch0.translations[2].time - ch0.translations[1].time;
    assert!((dt - dt2).abs() < 1e-5, "frame spacing should be uniform");

    // Events from annotation track
    assert_eq!(clip.events.len(), 1);
    assert_eq!(clip.events[0].text, "FootLeft");
    assert!((clip.events[0].time - 0.066667).abs() < 1e-4);
}

#[test]
fn interleaved_clip_with_skeleton_maps_bone_names() {
    let skeleton = parse_skeleton_xml(SKELETON_XML).expect("parse skeleton");
    let clip = extract_clip(INTERLEAVED_ANIMATION_XML, Some(&skeleton))
        .expect("extract interleaved clip with skeleton");

    assert_eq!(clip.channels[0].bone_name, "Root");
    assert_eq!(clip.channels[1].bone_name, "Spine");
}

// ---------------------------------------------------------------------------
// Test 3: Spline XML → static-rotation round-trip
//
// A hand-built 1-track, 1-frame spline buffer (static UNCOMPRESSED rotation,
// static translation/scale) embedded in hkaSplineCompressedAnimation XML.
// ---------------------------------------------------------------------------

fn build_static_spline_buf() -> Vec<u8> {
    // 1 track, 1 frame, 1 block, static UNCOMPRESSED rotation (identity)
    // packed_q: trans_q=0(BITS8), rot_q=5(UNCOMPRESSED), scale_q=0(BITS8)
    //   packed_q = 0 | (5 << 2) | (0 << 6) = 0x14
    // trans_mask = 0x07 (static x,y,z)
    // rot_mask   = 0x0F (static)
    // scale_mask = 0x07 (static x,y,z)
    //
    // Block layout (mask_and_quant_size=4):
    //   [0..4)   mask: [0x14, 0x07, 0x0F, 0x07]
    //   [4..16)  translation: (0.0, 0.0, 0.0) → 3 × f32le
    //   [16..32) rotation: (0.0, 0.0, 0.0, 1.0) → 4 × f32le (identity)
    //   [32..44) scale: (1.0, 1.0, 1.0) → 3 × f32le

    let mut buf: Vec<u8> = vec![0x14, 0x07, 0x0F, 0x07]; // mask block
    for v in [0.0f32, 0.0, 0.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0.0f32, 0.0, 0.0, 1.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in [1.0f32, 1.0, 1.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

fn make_spline_xml(data_bytes: &[u8]) -> String {
    let data_str: Vec<String> = data_bytes.iter().map(|b| b.to_string()).collect();
    let data_joined = data_str.join(" ");
    let data_len = data_bytes.len();

    format!(
        r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#anim" class="hkaSplineCompressedAnimation" signature="0x792ee0bb">
      <hkparam name="duration">0.033333</hkparam>
      <hkparam name="numberOfTransformTracks">1</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="numFrames">1</hkparam>
      <hkparam name="numBlocks">1</hkparam>
      <hkparam name="maxFramesPerBlock">1</hkparam>
      <hkparam name="maskAndQuantizationSize">4</hkparam>
      <hkparam name="blockDuration">0.033333</hkparam>
      <hkparam name="blockInverseDuration">30.0</hkparam>
      <hkparam name="frameDuration">0.033333</hkparam>
      <hkparam name="blockOffsets" numelements="1">0</hkparam>
      <hkparam name="floatBlockOffsets" numelements="0"></hkparam>
      <hkparam name="data" numelements="{data_len}">{data_joined}</hkparam>
      <hkparam name="annotationTracks" numelements="0"></hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##
    )
}

#[test]
fn spline_clip_static_rotation_extracts_identity_keyframe() {
    let buf = build_static_spline_buf();
    let xml = make_spline_xml(&buf);

    let clip = extract_clip(&xml, None).expect("extract spline clip");

    assert_eq!(clip.source_format, "hkx");
    assert_eq!(clip.channels.len(), 1);

    let ch = &clip.channels[0];
    assert_eq!(ch.bone_name, "track_0");

    // Should have at least 1 rotation keyframe with identity quaternion
    assert!(
        !ch.rotations.is_empty(),
        "spline clip should have rotation keyframe"
    );
    let r = &ch.rotations[0];
    assert_eq!(r.time, 0.0);
    assert!((r.value[0]).abs() < 1e-5, "x should be 0");
    assert!((r.value[1]).abs() < 1e-5, "y should be 0");
    assert!((r.value[2]).abs() < 1e-5, "z should be 0");
    assert!(
        (r.value[3] - 1.0).abs() < 1e-5,
        "w should be 1.0 (identity)"
    );

    // Translation should be (0,0,0)
    assert!(
        !ch.translations.is_empty(),
        "spline clip should have translation keyframe"
    );
    let t = &ch.translations[0];
    assert!((t.value[0]).abs() < 1e-5);
    assert!((t.value[1]).abs() < 1e-5);
    assert!((t.value[2]).abs() < 1e-5);
}

// ---------------------------------------------------------------------------
// Test 4: Quantized XML blob extraction
// ---------------------------------------------------------------------------

#[test]
fn quantized_clip_extracts_channels_from_inline_data_blob() {
    let data = build_synthetic_static_blob(2, 3, 1.0);
    let data_joined = data.iter().map(u8::to_string).collect::<Vec<_>>().join(" ");
    let data_len = data.len();
    let xml = format!(
        r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#anim" class="hkaQuantizedAnimation" signature="0x00000001">
      <hkparam name="duration">1.0</hkparam>
      <hkparam name="numberOfTransformTracks">2</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="numberOfFrames">3</hkparam>
      <hkparam name="data" numelements="{data_len}">{data_joined}</hkparam>
      <hkparam name="annotationTracks" numelements="0"></hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##
    );

    let clip = extract_clip(&xml, None).expect("extract quantized clip");

    assert_eq!(clip.channels.len(), 2);
    assert!(
        clip.warnings.is_empty(),
        "quantized extraction warnings: {:?}",
        clip.warnings
    );
    assert_eq!(clip.channels[0].bone_name, "track_0");
    assert_eq!(clip.channels[0].translations.len(), 3);
    assert_eq!(clip.channels[0].rotations.len(), 3);
    assert!((clip.channels[0].translations[0].value[0] - 1.0).abs() < 1e-5);
    assert!((clip.native_fps - 2.0).abs() < 1e-5);
}

// ---------------------------------------------------------------------------
// Test 5: infer_clip_fps
// ---------------------------------------------------------------------------

#[test]
fn infer_clip_fps_recovers_sample_rate_from_keyframe_spacing() {
    use havok_native::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};

    let fps = 30.0f32;
    let dt = 1.0f32 / fps;
    let clip = AnimationClip {
        source_format: "test".into(),
        duration: 1.0,
        native_fps: 0.0, // not yet set
        channels: vec![BoneChannel {
            bone_name: "Root".into(),
            translations: vec![
                AnimationKeyframe {
                    time: 0.0,
                    value: [0.0; 3],
                },
                AnimationKeyframe {
                    time: dt,
                    value: [1.0, 0.0, 0.0],
                },
            ],
            rotations: vec![],
            scales: vec![],
        }],
        events: vec![],
        original_skeleton_name: None,
        warnings: vec![],
        is_additive: false,
        track_to_bone_indices: vec![],
        extracted_motion_ref: String::new(),
    };

    let inferred = infer_clip_fps(&clip, 24.0);
    assert!(
        (inferred - fps).abs() < 0.5,
        "expected ~30 fps, got {inferred}"
    );
}

#[test]
fn infer_clip_fps_falls_back_to_default_for_single_keyframe_clip() {
    use havok_native::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};

    let clip = AnimationClip {
        source_format: "test".into(),
        duration: 0.033333,
        native_fps: 0.0,
        channels: vec![BoneChannel {
            bone_name: "Root".into(),
            translations: vec![AnimationKeyframe {
                time: 0.0,
                value: [0.0; 3],
            }],
            rotations: vec![],
            scales: vec![],
        }],
        events: vec![],
        original_skeleton_name: None,
        warnings: vec![],
        is_additive: false,
        track_to_bone_indices: vec![],
        extracted_motion_ref: String::new(),
    };

    let inferred = infer_clip_fps(&clip, 24.0);
    assert_eq!(inferred, 24.0, "should fall back to default 24.0");
}
