use havok_native::animation::parsers::{
    parse_animation_xml_str, parse_behavior_xml, parse_character_xml, parse_project_xml,
    parse_skeleton_xml,
};

// ---------------------------------------------------------------------------
// Skeleton XML fixtures
// ---------------------------------------------------------------------------

/// Packed hkQsTransform layout: 12 floats per bone (vec4 + quat + vec4).
/// The 4th component of translation and scale vectors is ignored padding.
const SKELETON_PACKED_XML: &str = r##"<?xml version="1.0"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject class="hkaSkeleton">
      <hkparam name="name">Test</hkparam>
      <hkparam name="parentIndices" numelements="2">-1 0</hkparam>
      <hkparam name="bones" numelements="2">
        <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
        <hkobject><hkparam name="name">Spine</hkparam><hkparam name="lockTranslation">1</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="2">
        (0 0 0 0 0 0 0 1 1 1 1 0)
        (0 1 0 0 0 0 0 1 1 1 1 0)
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

/// Legacy triplet layout: three separate groups per bone — (t)(q)(s).
const SKELETON_TRIPLET_XML: &str = r##"<?xml version="1.0"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject class="hkaSkeleton">
      <hkparam name="name">LegacySkel</hkparam>
      <hkparam name="parentIndices" numelements="1">-1</hkparam>
      <hkparam name="bones" numelements="1">
        <hkobject><hkparam name="name">Hip</hkparam><hkparam name="lockTranslation">false</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="1">
        (1 2 3)(0 0 0 1)(1 1 1)
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

/// Compact 10-float layout: (tx ty tz qx qy qz qw sx sy sz) per bone.
const SKELETON_COMPACT_XML: &str = r##"<?xml version="1.0"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject class="hkaSkeleton">
      <hkparam name="name">CompactSkel</hkparam>
      <hkparam name="parentIndices" numelements="1">-1</hkparam>
      <hkparam name="bones" numelements="1">
        <hkobject><hkparam name="name">Pelvis</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="1">
        (0.5 1.5 2.5 0 0 0 1 1 1 1)
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

/// Skeleton with float slots and reference floats.
const SKELETON_FLOAT_SLOTS_XML: &str = r##"<?xml version="1.0"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject class="hkaSkeleton">
      <hkparam name="name">FloatSkel</hkparam>
      <hkparam name="parentIndices" numelements="1">-1</hkparam>
      <hkparam name="bones" numelements="1">
        <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="1">
        (0 0 0 0 0 0 0 1 1 1 1 0)
      </hkparam>
      <hkparam name="floatSlots" numelements="2">
        <hkcstring>SlotA</hkcstring>
        <hkcstring>SlotB</hkcstring>
      </hkparam>
      <hkparam name="referenceFloats" numelements="2">0.5 1.0</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

/// Skeleton with partitions.
const SKELETON_PARTITIONS_XML: &str = r##"<?xml version="1.0"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject class="hkaSkeleton">
      <hkparam name="name">PartitionSkel</hkparam>
      <hkparam name="parentIndices" numelements="1">-1</hkparam>
      <hkparam name="bones" numelements="1">
        <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="1">
        (0 0 0 0 0 0 0 1 1 1 1 0)
      </hkparam>
      <hkparam name="partitions" numelements="2">
        <hkobject><hkparam name="name">Upper</hkparam></hkobject>
        <hkobject><hkparam name="name">Lower</hkparam></hkobject>
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

// ---------------------------------------------------------------------------
// Animation XML fixtures
// ---------------------------------------------------------------------------

const LOSSLESS_ANIMATION_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#animation" class="hkaLosslessCompressedAnimation" signature="0x6b5b1a01">
      <hkparam name="duration">0.5</hkparam>
      <hkparam name="numberOfTransformTracks">2</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="staticRotations" numelements="2">
        (0 0 0 1)
        (0.707 0 0 0.707)
      </hkparam>
      <hkparam name="staticTranslations" numelements="2">
        (0 0 0)
        (0 10 0)
      </hkparam>
      <hkparam name="rotationTypeAndOffsets" numelements="2">4 4</hkparam>
      <hkparam name="translationTypeAndOffsets" numelements="2">4 4</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

const INTERLEAVED_ANIMATION_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#animation" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
      <hkparam name="duration">1.0</hkparam>
      <hkparam name="numberOfTransformTracks">2</hkparam>
      <hkparam name="numberOfFloatTracks">1</hkparam>
      <hkparam name="transforms" numelements="4">
        (0 0 0 0 0 0 0 1 1 1 1 0)
        (1 0 0 0 0 0 0 1 1 1 1 0)
        (2 0 0 0 0 0 0 1 1 1 1 0)
        (3 0 0 0 0 0 0 1 1 1 1 0)
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

// ---------------------------------------------------------------------------
// Behavior XML fixture
// ---------------------------------------------------------------------------

const BEHAVIOR_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbBehaviorGraphStringData">
      <hkparam name="eventNames" numelements="2">
        <hkcstring>footstep</hkcstring>
        <hkcstring>attack</hkcstring>
      </hkparam>
      <hkparam name="variableNames" numelements="2">
        <hkcstring>speed</hkcstring>
        <hkcstring>isAttacking</hkcstring>
      </hkparam>
    </hkobject>
    <hkobject name="#0002" class="hkbBehaviorGraphData">
      <hkparam name="variableInfos" numelements="2">
        <hkobject>
          <hkparam name="type">VARIABLE_TYPE_REAL</hkparam>
        </hkobject>
        <hkobject>
          <hkparam name="type">VARIABLE_TYPE_BOOL</hkparam>
        </hkobject>
      </hkparam>
    </hkobject>
    <hkobject name="#0003" class="BGSGamebryoSequenceGenerator">
      <hkparam name="pSequence">idle.hkx</hkparam>
    </hkobject>
    <hkobject name="#0004" class="hkbBlendingTransitionEffect">
      <hkparam name="name">BlendToIdle</hkparam>
      <hkparam name="duration">0.2</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

// ---------------------------------------------------------------------------
// Character XML fixture
// ---------------------------------------------------------------------------

const CHARACTER_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbCharacterStringData">
      <hkparam name="rigName">Actors\Character\Character Assets\skeleton.hkx</hkparam>
      <hkparam name="behaviorFilename">Actors\Character\Behaviors\0_master.hkx</hkparam>
    </hkobject>
    <hkobject name="#0002" class="hkbCharacterData">
      <hkparam name="modelUpMS">(0 0 1 0)</hkparam>
      <hkparam name="modelForwardMS">(1 0 0 0)</hkparam>
      <hkparam name="modelRightMS">(0 1 0 0)</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

// ---------------------------------------------------------------------------
// Project XML fixture
// ---------------------------------------------------------------------------

const PROJECT_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbProjectStringData">
      <hkparam name="characterFilenames" numelements="2">
        <hkcstring>Actors\Character\character.hkx</hkcstring>
        <hkcstring>Actors\Dog\character.hkx</hkcstring>
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;

// ===========================================================================
// Skeleton tests
// ===========================================================================

#[test]
fn parses_skeleton_with_packed_qs_transform_layout() {
    let s = parse_skeleton_xml(SKELETON_PACKED_XML).unwrap();
    assert_eq!(s.name, "Test");
    assert_eq!(s.bone_names, vec!["Root".to_string(), "Spine".to_string()]);
    assert_eq!(s.parent_indices, vec![-1, 0]);
    assert_eq!(s.lock_translation, vec![false, true]);
    assert_eq!(s.reference_pose.len(), 2);
    // Bone 0: (0 0 0 0 | 0 0 0 1 | 1 1 1 0) → t=[0,0,0], q=[0,0,0,1], s=[1,1,1]
    assert_eq!(s.reference_pose[0].t, [0.0, 0.0, 0.0]);
    assert_eq!(s.reference_pose[0].q, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.reference_pose[0].s, [1.0, 1.0, 1.0]);
    // Bone 1: (0 1 0 0 | 0 0 0 1 | 1 1 1 0) → t=[0,1,0], q=[0,0,0,1], s=[1,1,1]
    assert_eq!(s.reference_pose[1].t, [0.0, 1.0, 0.0]);
    assert_eq!(s.reference_pose[1].q, [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn parses_skeleton_with_legacy_triplet_layout() {
    let s = parse_skeleton_xml(SKELETON_TRIPLET_XML).unwrap();
    assert_eq!(s.name, "LegacySkel");
    assert_eq!(s.bone_names, vec!["Hip".to_string()]);
    assert_eq!(s.reference_pose.len(), 1);
    assert_eq!(s.reference_pose[0].t, [1.0, 2.0, 3.0]);
    assert_eq!(s.reference_pose[0].q, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.reference_pose[0].s, [1.0, 1.0, 1.0]);
    assert_eq!(s.lock_translation[0], false);
}

#[test]
fn parses_skeleton_with_compact_10float_layout() {
    let s = parse_skeleton_xml(SKELETON_COMPACT_XML).unwrap();
    assert_eq!(s.name, "CompactSkel");
    assert_eq!(s.reference_pose.len(), 1);
    assert!((s.reference_pose[0].t[0] - 0.5).abs() < 1e-6);
    assert!((s.reference_pose[0].t[1] - 1.5).abs() < 1e-6);
    assert!((s.reference_pose[0].t[2] - 2.5).abs() < 1e-6);
    assert_eq!(s.reference_pose[0].q, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.reference_pose[0].s, [1.0, 1.0, 1.0]);
}

#[test]
fn parses_skeleton_float_slots_and_reference_floats() {
    let s = parse_skeleton_xml(SKELETON_FLOAT_SLOTS_XML).unwrap();
    assert_eq!(
        s.float_slots,
        vec!["SlotA".to_string(), "SlotB".to_string()]
    );
    assert_eq!(s.float_count, 2);
    assert!((s.reference_floats[0] - 0.5).abs() < 1e-6);
    assert!((s.reference_floats[1] - 1.0).abs() < 1e-6);
}

#[test]
fn parses_skeleton_partitions() {
    let s = parse_skeleton_xml(SKELETON_PARTITIONS_XML).unwrap();
    assert_eq!(
        s.partition_names,
        vec!["Upper".to_string(), "Lower".to_string()]
    );
}

#[test]
fn skeleton_lock_translation_pads_to_bone_count() {
    // Bones array has 2 elements but only first has lockTranslation specified.
    // Parser must pad remaining with false.
    let xml = r##"<?xml version="1.0"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject class="hkaSkeleton">
      <hkparam name="name">PadTest</hkparam>
      <hkparam name="parentIndices" numelements="2">-1 0</hkparam>
      <hkparam name="bones" numelements="2">
        <hkobject><hkparam name="name">A</hkparam><hkparam name="lockTranslation">1</hkparam></hkobject>
        <hkobject><hkparam name="name">B</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="2">
        (0 0 0 0 0 0 0 1 1 1 1 0)
        (0 0 0 0 0 0 0 1 1 1 1 0)
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"##;
    let s = parse_skeleton_xml(xml).unwrap();
    assert_eq!(s.lock_translation.len(), 2);
    assert_eq!(s.lock_translation[0], true);
    assert_eq!(s.lock_translation[1], false);
}

#[test]
fn skeleton_parser_returns_error_on_malformed_xml() {
    let result = parse_skeleton_xml("<broken<<xml");
    assert!(result.is_err());
}

// ===========================================================================
// Animation tests
// ===========================================================================

#[test]
fn parses_lossless_animation_metadata() {
    let a = parse_animation_xml_str(LOSSLESS_ANIMATION_XML).unwrap();
    assert_eq!(a.compression_type, "lossless");
    assert!((a.duration - 0.5).abs() < 1e-6);
    assert_eq!(a.bone_count, 2);
    assert_eq!(a.float_track_count, 0);
}

#[test]
fn lossless_animation_frame0_decodes_static_transforms() {
    let a = parse_animation_xml_str(LOSSLESS_ANIMATION_XML).unwrap();
    // rotationTypeAndOffsets: 4 4 → type=0 (identity), offset=1 (static)
    // The static type+offset encoding: value & 3 = type, value >> 2 = index
    // 4 & 3 = 0 (identity), 4 >> 2 = 1 → identity rotation for both bones
    let blob = a
        .frame0_transforms
        .expect("frame0 blob present for lossless animation");
    // Each bone is 7 float32 values: qx,qy,qz,qw,tx,ty,tz
    assert_eq!(blob.len(), 2 * 7 * 4);
    // Parse first bone (identity rotation, identity translation)
    let q: Vec<f32> = blob[0..16]
        .chunks(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    // rotationTypeAndOffsets = 4 → type = 0 (identity), not type 1 (static)
    // so rotation should be identity [0,0,0,1]
    assert!(
        (q[3] - 1.0).abs() < 1e-5,
        "identity rotation qw should be 1.0, got {}",
        q[3]
    );
}

#[test]
fn parses_interleaved_animation_metadata() {
    let a = parse_animation_xml_str(INTERLEAVED_ANIMATION_XML).unwrap();
    assert_eq!(a.compression_type, "interleaved");
    assert!((a.duration - 1.0).abs() < 1e-6);
    assert_eq!(a.bone_count, 2);
    assert_eq!(a.float_track_count, 1);
    // frame0_transforms is None for interleaved (not a lossless animation)
    assert!(a.frame0_transforms.is_none());
}

#[test]
fn animation_parser_returns_error_on_malformed_xml() {
    let result = parse_animation_xml_str("<broken<<xml");
    assert!(result.is_err());
}

// ===========================================================================
// Behavior tests
// ===========================================================================

#[test]
fn parses_behavior_events_and_variables() {
    let b = parse_behavior_xml(BEHAVIOR_XML).unwrap();
    assert_eq!(b.events, vec!["footstep".to_string(), "attack".to_string()]);
    assert_eq!(b.variables[0].0, "speed");
    assert_eq!(b.variables[1].0, "isAttacking");
}

#[test]
fn behavior_variable_types_patched_from_graph_data() {
    let b = parse_behavior_xml(BEHAVIOR_XML).unwrap();
    // hkbBehaviorGraphData variableInfos patch the types
    assert_eq!(b.variables[0].1, "VARIABLE_TYPE_REAL");
    assert_eq!(b.variables[1].1, "VARIABLE_TYPE_BOOL");
}

#[test]
fn parses_behavior_sequences_and_transitions() {
    let b = parse_behavior_xml(BEHAVIOR_XML).unwrap();
    assert_eq!(b.sequences, vec!["idle.hkx".to_string()]);
    assert_eq!(b.transitions.len(), 1);
    assert_eq!(b.transitions[0].0, "BlendToIdle");
    assert_eq!(b.transitions[0].1, "0.2");
}

#[test]
fn behavior_node_count_and_classes_collected() {
    let b = parse_behavior_xml(BEHAVIOR_XML).unwrap();
    assert!(b.node_count >= 4);
    assert!(
        b.node_classes
            .contains(&"hkbBehaviorGraphStringData".to_string())
    );
    assert!(b.node_classes.contains(&"hkbBehaviorGraphData".to_string()));
    assert!(
        b.node_classes
            .contains(&"BGSGamebryoSequenceGenerator".to_string())
    );
}

#[test]
fn behavior_parser_returns_error_on_malformed_xml() {
    let result = parse_behavior_xml("<broken<<xml");
    assert!(result.is_err());
}

// ===========================================================================
// Character tests
// ===========================================================================

#[test]
fn parses_character_rig_and_behavior_filenames() {
    let c = parse_character_xml(CHARACTER_XML).unwrap();
    assert_eq!(
        c.rig_name,
        "Actors\\Character\\Character Assets\\skeleton.hkx"
    );
    assert_eq!(
        c.behavior_filename,
        "Actors\\Character\\Behaviors\\0_master.hkx"
    );
}

#[test]
fn parses_character_model_axes() {
    let c = parse_character_xml(CHARACTER_XML).unwrap();
    assert_eq!(c.model_up, "(0 0 1 0)");
    assert_eq!(c.model_forward, "(1 0 0 0)");
    assert_eq!(c.model_right, "(0 1 0 0)");
}

#[test]
fn character_parser_returns_error_on_malformed_xml() {
    let result = parse_character_xml("<broken<<xml");
    assert!(result.is_err());
}

// ===========================================================================
// Project tests
// ===========================================================================

#[test]
fn parses_project_character_filenames() {
    let p = parse_project_xml(PROJECT_XML).unwrap();
    assert_eq!(
        p.character_filenames,
        vec![
            "Actors\\Character\\character.hkx".to_string(),
            "Actors\\Dog\\character.hkx".to_string(),
        ]
    );
}

#[test]
fn project_parser_returns_error_on_malformed_xml() {
    let result = parse_project_xml("<broken<<xml");
    assert!(result.is_err());
}
