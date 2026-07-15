use indexmap::IndexMap;
use nif_core_native::convert_file::{ConvertFileOptions, ConvertFileReport};
use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::skin::bone_remap::{
    BodyPartRemap, BoneEntry, SkeletonMap, VertexInfluences, fo3_body_part_to_fo4_segment,
    redistribute_unmapped,
};
use nif_core_native::skin::pack::{
    pack_skinned_vertex_data, recompute_tangents_lengyel, vertex_desc_skinned,
};
use nif_core_native::skin::segment::{SegmentSpec, build_segment_data};
use nif_core_native::skin::source::{
    LegacyPartition, LegacySkinKind, SkinTransform, fold_partitions_to_global, parse_skin_chain,
};
use nif_core_native::skin::weight_transfer::{MorphTransferConfig, transfer_morph_weights};
use nif_core_native::skin::{
    convert_legacy_skin, first_person::extract_arm_subset, restructure_bone_tree,
};
use std::path::PathBuf;

fn translation_maps_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conversion/src/embedded/translation_maps")
}

#[test]
fn options_have_skin_fields() {
    let opts = ConvertFileOptions {
        translation_maps_dir: None,
        auto_skin_reference_body: None,
        emit_first_person: false,
        first_person_reference: None,
        morph_weight_cap: 0.5,
        ..ConvertFileOptions::default()
    };

    assert!(!opts.emit_first_person);
    assert!((opts.morph_weight_cap - 0.5).abs() < f32::EPSILON);
}

#[test]
fn report_has_skin_counters() {
    let report = ConvertFileReport::default();

    assert_eq!(report.shapes_skinned, 0);
    assert_eq!(report.vertices_repacked, 0);
    assert!(report.emitted_first_person.is_none());
}

#[test]
fn skeleton_map_loads_fnv_to_fo4() {
    let dir = translation_maps_dir();
    let map = SkeletonMap::load(&dir, "fnv", "fo4").expect("load fnv->fo4 map");

    assert_eq!(map.lookup("Bip01 Pelvis"), Some("Pelvis"));
    assert_eq!(map.lookup("Bip01 Spine"), Some("Spine1"));
    assert_eq!(map.lookup("Bip01 L Forearm"), Some("LArm_ForeArm1"));
}

#[test]
fn skeleton_map_returns_none_for_unmapped() {
    let dir = translation_maps_dir();
    let map = SkeletonMap::load(&dir, "fnv", "fo4").expect("load fnv->fo4 map");

    assert_eq!(map.lookup("Bip01 BogusBone"), None);
}

#[test]
fn skeleton_map_missing_file_errors() {
    let dir = PathBuf::from("/nonexistent/path");
    let result = SkeletonMap::load(&dir, "fnv", "fo4");

    assert!(result.is_err());
}

#[test]
fn fo3_body_part_torso_maps_to_fo4_body() {
    let remap = fo3_body_part_to_fo4_segment(0);

    assert_eq!(
        remap,
        Some(BodyPartRemap {
            fo4_partition: 32,
            segment_user_index: 32
        })
    );
}

#[test]
fn fo3_body_part_left_arm_maps_to_fo4_arms() {
    let remap = fo3_body_part_to_fo4_segment(2);

    assert_eq!(
        remap,
        Some(BodyPartRemap {
            fo4_partition: 34,
            segment_user_index: 34
        })
    );
}

#[test]
fn fo3_body_part_unknown_returns_none() {
    assert_eq!(fo3_body_part_to_fo4_segment(255), None);
}

#[test]
fn unmapped_child_weight_redistributes_to_mapped_parent() {
    let bones = vec![
        BoneEntry {
            name: "Bip01 Pelvis".into(),
            parent: -1,
        },
        BoneEntry {
            name: "Bip01 BogusBone".into(),
            parent: 0,
        },
    ];
    let mut influences = vec![VertexInfluences {
        slots: vec![(0, 0.6), (1, 0.4)],
    }];

    let dir = translation_maps_dir();
    let map = SkeletonMap::load(&dir, "fnv", "fo4").expect("load map");
    let report = redistribute_unmapped(&mut influences, &bones, &map);

    assert_eq!(influences[0].slots.len(), 1);
    assert_eq!(influences[0].slots[0].0, 0);
    assert!((influences[0].slots[0].1 - 1.0).abs() < 1e-5);
    assert_eq!(report.dropped_unmapped, vec!["Bip01 BogusBone"]);
    assert_eq!(report.weights_redistributed, 1);
}

#[test]
fn fully_unmapped_chain_drops_to_root_with_warning() {
    let bones = vec![
        BoneEntry {
            name: "BogusRoot".into(),
            parent: -1,
        },
        BoneEntry {
            name: "BogusChild".into(),
            parent: 0,
        },
    ];
    let mut influences = vec![VertexInfluences {
        slots: vec![(0, 0.5), (1, 0.5)],
    }];

    let dir = translation_maps_dir();
    let map = SkeletonMap::load(&dir, "fnv", "fo4").expect("load map");
    let report = redistribute_unmapped(&mut influences, &bones, &map);

    assert!(influences[0].slots.is_empty());
    assert_eq!(report.dropped_unmapped.len(), 2);
}

fn make_minimal_skinned_nif() -> (NifFile, usize) {
    let mut nif = NifFile::new("fnv");

    let mut pelvis = IndexMap::new();
    pelvis.insert("Name".into(), NifValue::String("Bip01 Pelvis".into()));
    let pelvis_id = nif.add_block("NiNode", Some(pelvis));

    let mut spine = IndexMap::new();
    spine.insert("Name".into(), NifValue::String("Bip01 Spine".into()));
    let spine_id = nif.add_block("NiNode", Some(spine));
    let pelvis = nif.blocks.get_mut(pelvis_id).expect("pelvis node");
    pelvis.set_field("Num Children", NifValue::UInt(1));
    pelvis.set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(spine_id as i32)]),
    );

    let mut skin_data = IndexMap::new();
    skin_data.insert("Num Bones".into(), NifValue::UInt(2));
    skin_data.insert("Has Vertex Weights".into(), NifValue::Bool(true));
    let skin_data_id = nif.add_block("NiSkinData", Some(skin_data));

    let mut skin_partition = IndexMap::new();
    skin_partition.insert("Num Partitions".into(), NifValue::UInt(0));
    skin_partition.insert("Partitions".into(), NifValue::Array(Vec::new()));
    let skin_partition_id = nif.add_block("NiSkinPartition", Some(skin_partition));

    let mut skin_instance = IndexMap::new();
    skin_instance.insert("Data".into(), NifValue::Ref(skin_data_id as i32));
    skin_instance.insert(
        "Skin Partition".into(),
        NifValue::Ref(skin_partition_id as i32),
    );
    skin_instance.insert("Skeleton Root".into(), NifValue::Ref(0));
    skin_instance.insert("Num Bones".into(), NifValue::UInt(2));
    skin_instance.insert(
        "Bones".into(),
        NifValue::Array(vec![
            NifValue::Ref(pelvis_id as i32),
            NifValue::Ref(spine_id as i32),
        ]),
    );
    let skin_instance_id = nif.add_block("NiSkinInstance", Some(skin_instance));

    let mut shape = IndexMap::new();
    shape.insert("Name".into(), NifValue::String("UpperBody".into()));
    shape.insert(
        "Skin Instance".into(),
        NifValue::Ref(skin_instance_id as i32),
    );
    let shape_id = nif.add_block("NiTriShape", Some(shape));

    (nif, shape_id)
}

fn make_convertible_legacy_skin_nif() -> (NifFile, usize) {
    let (mut nif, shape_id) = make_minimal_skinned_nif();
    let vertex_data = vec![
        vertex_entry([0.0, 0.0, 0.0]),
        vertex_entry([1.0, 0.0, 0.0]),
        vertex_entry([0.0, 1.0, 0.0]),
    ];
    let triangles = vec![triangle_value([0, 1, 2])];
    if let Some(shape) = nif.blocks.get_mut(shape_id) {
        shape.set_field("Vertex Data", NifValue::Array(vertex_data));
        shape.set_field("Triangles", NifValue::Array(triangles));
        shape.set_field("Num Vertices", NifValue::UInt(3));
        shape.set_field("Num Triangles", NifValue::UInt(1));
    }

    let instance_id = match nif
        .get_block(shape_id)
        .and_then(|shape| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected skin instance ref: {other:?}"),
    };
    let partition_id = match nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Skin Partition"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected partition ref: {other:?}"),
    };
    let partition = NifValue::Struct(fields([
        ("Body Part", NifValue::UInt(0)),
        (
            "Vertex Map",
            NifValue::Array(vec![
                NifValue::UInt(0),
                NifValue::UInt(1),
                NifValue::UInt(2),
            ]),
        ),
        (
            "Bone Indices",
            NifValue::Array(vec![
                NifValue::Array(vec![
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ]),
                NifValue::Array(vec![
                    NifValue::UInt(1),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ]),
                NifValue::Array(vec![
                    NifValue::UInt(1),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ]),
            ]),
        ),
        (
            "Vertex Weights",
            NifValue::Array(vec![
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ]),
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ]),
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ]),
            ]),
        ),
        (
            "Bones",
            NifValue::Array(vec![NifValue::UInt(0), NifValue::UInt(1)]),
        ),
        (
            "Triangles",
            NifValue::Array(vec![triangle_value([0, 1, 2])]),
        ),
    ]));
    let partition_block = nif.blocks.get_mut(partition_id).expect("partition block");
    partition_block.set_field("Num Partitions", NifValue::UInt(1));
    partition_block.set_field("Partitions", NifValue::Array(vec![partition]));

    (nif, shape_id)
}

fn add_convertible_shape_sharing_skin_bones(nif: &mut NifFile, shape_id: usize) -> usize {
    let instance_id = skin_instance_id(nif, shape_id);
    let bones = nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Bones"))
        .cloned()
        .expect("bones");
    let mut skin_data = IndexMap::new();
    skin_data.insert("Num Bones".into(), NifValue::UInt(2));
    skin_data.insert("Has Vertex Weights".into(), NifValue::Bool(true));
    let skin_data_id = nif.add_block("NiSkinData", Some(skin_data));

    let partition = NifValue::Struct(fields([
        ("Body Part", NifValue::UInt(0)),
        (
            "Vertex Map",
            NifValue::Array(vec![
                NifValue::UInt(0),
                NifValue::UInt(1),
                NifValue::UInt(2),
            ]),
        ),
        (
            "Bone Indices",
            NifValue::Array(vec![
                NifValue::Array(vec![
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ]),
                NifValue::Array(vec![
                    NifValue::UInt(1),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ]),
                NifValue::Array(vec![
                    NifValue::UInt(1),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ]),
            ]),
        ),
        (
            "Vertex Weights",
            NifValue::Array(vec![
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ]),
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ]),
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ]),
            ]),
        ),
        (
            "Bones",
            NifValue::Array(vec![NifValue::UInt(0), NifValue::UInt(1)]),
        ),
        (
            "Triangles",
            NifValue::Array(vec![triangle_value([0, 1, 2])]),
        ),
    ]));
    let skin_partition_id = nif.add_block(
        "NiSkinPartition",
        Some(fields([
            ("Num Partitions", NifValue::UInt(1)),
            ("Partitions", NifValue::Array(vec![partition])),
        ])),
    );

    let skin_instance_id = nif.add_block(
        "NiSkinInstance",
        Some(fields([
            ("Data", NifValue::Ref(skin_data_id as i32)),
            ("Skin Partition", NifValue::Ref(skin_partition_id as i32)),
            ("Skeleton Root", NifValue::Ref(0)),
            ("Num Bones", NifValue::UInt(2)),
            ("Bones", bones),
        ])),
    );
    nif.add_block(
        "NiTriShape",
        Some(fields([
            ("Name", NifValue::String("LowerBody".into())),
            ("Skin Instance", NifValue::Ref(skin_instance_id as i32)),
            (
                "Vertex Data",
                NifValue::Array(vec![
                    vertex_entry([0.0, 0.0, 1.0]),
                    vertex_entry([1.0, 0.0, 1.0]),
                    vertex_entry([0.0, 1.0, 1.0]),
                ]),
            ),
            (
                "Triangles",
                NifValue::Array(vec![triangle_value([0, 1, 2])]),
            ),
            ("Num Vertices", NifValue::UInt(3)),
            ("Num Triangles", NifValue::UInt(1)),
        ])),
    )
}

fn skin_instance_id(nif: &NifFile, shape_id: usize) -> usize {
    match nif
        .get_block(shape_id)
        .and_then(|shape| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected skin instance ref: {other:?}"),
    }
}

fn skin_data_id(nif: &NifFile, shape_id: usize) -> usize {
    let instance_id = skin_instance_id(nif, shape_id);
    match nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Data"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected skin data ref: {other:?}"),
    }
}

fn skin_partition_id(nif: &NifFile, shape_id: usize) -> usize {
    let instance_id = skin_instance_id(nif, shape_id);
    match nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Skin Partition"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected partition ref: {other:?}"),
    }
}

fn skin_transform_value(translation: [f32; 3]) -> NifValue {
    NifValue::Struct(fields([
        ("Translation", NifValue::Vec3(translation)),
        (
            "Rotation",
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        ),
        ("Scale", NifValue::Float(1.0)),
    ]))
}

fn bone_data_value(translation: [f32; 3], weights: Vec<NifValue>) -> NifValue {
    NifValue::Struct(fields([
        ("Skin Transform", skin_transform_value(translation)),
        ("Vertex Weights", NifValue::Array(weights)),
    ]))
}

fn bone_weight_value(vertex_index: u32, weight: f32) -> NifValue {
    NifValue::Struct(fields([
        ("Index", NifValue::UInt(vertex_index as u64)),
        ("Weight", NifValue::Float(weight as f64)),
    ]))
}

#[test]
fn parse_skin_chain_captures_kind_and_bones() {
    let (nif, shape_id) = make_minimal_skinned_nif();

    let parsed = parse_skin_chain(&nif, shape_id)
        .expect("parse succeeds")
        .expect("skin chain exists");
    let expected_instance = match nif
        .get_block(shape_id)
        .and_then(|shape| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected skin instance ref: {other:?}"),
    };
    let expected_data = match nif
        .get_block(expected_instance)
        .and_then(|instance| instance.get_field("Data"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected skin data ref: {other:?}"),
    };
    let expected_partition = match nif
        .get_block(expected_instance)
        .and_then(|instance| instance.get_field("Skin Partition"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => Some(*reference as usize),
        other => panic!("unexpected skin partition ref: {other:?}"),
    };

    assert_eq!(parsed.kind, LegacySkinKind::NonArmor);
    assert_eq!(parsed.instance_block_id, expected_instance);
    assert_eq!(parsed.skin_data_block_id, expected_data);
    assert_eq!(parsed.skin_partition_block_id, expected_partition);
    assert_eq!(parsed.bones.len(), 2);
    assert_eq!(parsed.bones[0].name, "Bip01 Pelvis");
    assert_eq!(parsed.bones[1].name, "Bip01 Spine");
    assert_eq!(parsed.bones[0].parent, -1);
    assert_eq!(parsed.bones[1].parent, 0);
    assert_eq!(parsed.skin_transform, SkinTransform::identity());
}

#[test]
fn parse_skin_chain_reads_skin_data_bind_weights() {
    let (mut nif, shape_id) = make_minimal_skinned_nif();
    let skin_data_id = skin_data_id(&nif, shape_id);
    let skin_data = nif.blocks.get_mut(skin_data_id).expect("skin data");
    skin_data.set_field("Skin Transform", skin_transform_value([2.0, 0.0, 0.0]));
    skin_data.set_field(
        "Bone List",
        NifValue::Array(vec![
            bone_data_value(
                [1.0, 0.0, 0.0],
                vec![bone_weight_value(0, 0.25), bone_weight_value(1, 1.0)],
            ),
            bone_data_value([0.0, 3.0, 0.0], vec![bone_weight_value(0, 0.75)]),
        ]),
    );

    let parsed = parse_skin_chain(&nif, shape_id)
        .expect("parse succeeds")
        .expect("skin chain exists");

    assert_eq!(parsed.skin_transform.translation, [2.0, 0.0, 0.0]);
    assert_eq!(parsed.bone_transforms[0].translation, [1.0, 0.0, 0.0]);
    assert_eq!(parsed.bone_transforms[1].translation, [0.0, 3.0, 0.0]);
    assert_eq!(parsed.data_influences[0].slots, vec![(0, 0.25), (1, 0.75)]);
    assert_eq!(parsed.data_influences[1].slots, vec![(0, 1.0)]);
}

#[test]
fn parse_skin_chain_detects_dismember_armor() {
    let (mut nif, shape_id) = make_minimal_skinned_nif();
    let instance_id = match nif
        .get_block(shape_id)
        .and_then(|shape| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected skin instance ref: {other:?}"),
    };
    let instance = nif.blocks.get_mut(instance_id).expect("instance exists");
    instance.type_name = "BSDismemberSkinInstance".to_string();

    let parsed = parse_skin_chain(&nif, shape_id)
        .expect("parse succeeds")
        .expect("skin chain exists");

    assert_eq!(parsed.kind, LegacySkinKind::Armor);
}

#[test]
fn parse_skin_chain_returns_none_for_unskinned_shape() {
    let mut nif = NifFile::new("fnv");
    let mut shape = IndexMap::new();
    shape.insert("Name".into(), NifValue::String("StaticShape".into()));
    shape.insert("Skin Instance".into(), NifValue::Ref(-1));
    let shape_id = nif.add_block("NiTriShape", Some(shape));

    let parsed = parse_skin_chain(&nif, shape_id).expect("parse succeeds");

    assert!(parsed.is_none());
}

#[test]
fn parse_skin_chain_accepts_fo4_skin_field_when_skin_instance_is_empty() {
    let (mut nif, shape_id) = make_minimal_skinned_nif();
    let instance_id = match nif
        .get_block(shape_id)
        .and_then(|shape| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference,
        other => panic!("unexpected skin instance ref: {other:?}"),
    };
    let shape = nif.blocks.get_mut(shape_id).expect("shape");
    shape.set_field("Skin Instance", NifValue::Ref(-1));
    shape.set_field("Skin", NifValue::Ref(instance_id));

    let parsed = parse_skin_chain(&nif, shape_id)
        .expect("parse succeeds")
        .expect("skin chain exists");

    assert_eq!(parsed.bones.len(), 2);
}

#[test]
fn fold_partitions_merges_overlapping_vertex_maps() {
    let parts = vec![
        LegacyPartition {
            body_part: 0,
            vertex_map: vec![0, 1, 2],
            influences: vec![vec![(0, 1.0)], vec![(0, 0.5), (1, 0.5)], vec![(0, 1.0)]],
            bones: vec![10, 11],
            triangles: Vec::new(),
        },
        LegacyPartition {
            body_part: 0,
            vertex_map: vec![2, 3, 4],
            influences: vec![vec![(0, 1.0)], vec![(1, 1.0)], vec![(0, 0.7), (1, 0.3)]],
            bones: vec![11, 12],
            triangles: Vec::new(),
        },
    ];

    let folded = fold_partitions_to_global(&parts, 5);

    assert_eq!(folded.len(), 5);
    assert_eq!(folded[0].slots, vec![(10, 1.0)]);
    assert_eq!(folded[1].slots, vec![(10, 0.5), (11, 0.5)]);
    let vertex_two = &folded[2].slots;
    assert_eq!(vertex_two.len(), 2);
    let total: f32 = vertex_two.iter().map(|(_, weight)| *weight).sum();
    assert!((total - 1.0).abs() < 1e-5);
    assert!(
        vertex_two
            .iter()
            .any(|(bone, weight)| *bone == 10 && (*weight - 0.5).abs() < 1e-5)
    );
    assert!(
        vertex_two
            .iter()
            .any(|(bone, weight)| *bone == 11 && (*weight - 0.5).abs() < 1e-5)
    );
}

#[test]
fn fold_clamps_to_four_influences_keeping_top_weights() {
    let parts = vec![LegacyPartition {
        body_part: 0,
        vertex_map: vec![0],
        influences: vec![vec![(0, 0.05), (1, 0.10), (2, 0.20), (3, 0.30), (4, 0.35)]],
        bones: vec![100, 101, 102, 103, 104],
        triangles: Vec::new(),
    }];

    let folded = fold_partitions_to_global(&parts, 1);

    assert_eq!(folded[0].slots.len(), 4);
    let total: f32 = folded[0].slots.iter().map(|(_, weight)| *weight).sum();
    assert!((total - 1.0).abs() < 1e-5);
    assert!(folded[0].slots.iter().all(|(bone, _)| *bone != 100));
}

#[test]
fn vertex_desc_skinned_sets_skinned_and_fullprecision_bits() {
    let desc = vertex_desc_skinned(false);
    let stride = (desc & 0xF) as u32;
    assert_eq!(stride, 7);
    let flags = (desc >> 44) as u32;
    assert!(flags & 0x40 != 0);
    assert!(flags & 0x4000 != 0);
}

#[test]
fn pack_skinned_vertex_data_writes_bone_indices_and_weights() {
    let positions = vec![[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0]];
    let normals = vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
    let tangents = vec![[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    let bitangents = vec![[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    let uvs = vec![[0.0_f32, 0.0], [1.0, 0.0]];
    let influences = vec![
        VertexInfluences {
            slots: vec![(0, 1.0)],
        },
        VertexInfluences {
            slots: vec![(0, 0.5), (1, 0.5)],
        },
    ];

    let entries = pack_skinned_vertex_data(
        &positions,
        &normals,
        &tangents,
        &bitangents,
        &uvs,
        None,
        &influences,
    );

    assert_eq!(entries.len(), 2);
    for entry in &entries {
        let NifValue::Struct(fields) = entry else {
            panic!("not a struct");
        };
        let bi = fields.get("Bone Indices").expect("Bone Indices");
        let bw = fields.get("Bone Weights").expect("Bone Weights");
        assert!(matches!(bi, NifValue::Array(a) if a.len() == 4));
        assert!(matches!(bw, NifValue::Array(a) if a.len() == 4));
    }
}

#[test]
fn tangents_for_simple_xy_quad_align_with_uv_axes() {
    let positions = vec![
        [0.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let normals = vec![[0.0_f32, 0.0, 1.0]; 4];
    let uvs = vec![[0.0_f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let triangles = vec![[0u32, 1, 2], [0, 2, 3]];

    let (tangents, bitangents) = recompute_tangents_lengyel(&positions, &normals, &uvs, &triangles);
    assert_eq!(tangents.len(), 4);
    for tangent in &tangents {
        assert!(tangent[0] > 0.9, "expected +X tangent, got {:?}", tangent);
    }
    for bitangent in &bitangents {
        assert!(
            bitangent[1] > 0.9,
            "expected +Y bitangent, got {:?}",
            bitangent
        );
    }
}

#[test]
fn build_segment_data_emits_one_segment_per_input() {
    let specs = vec![
        SegmentSpec {
            triangle_start: 0,
            triangle_count: 100,
            user_index: 32,
        },
        SegmentSpec {
            triangle_start: 100,
            triangle_count: 50,
            user_index: 34,
        },
    ];
    let (num_segments, segments_value, total_segment_data) = build_segment_data(&specs);
    assert_eq!(num_segments, 2);

    let arr = match &segments_value {
        NifValue::Array(items) => items,
        _ => panic!("not an array"),
    };
    assert_eq!(arr.len(), 2);
    let first = match &arr[0] {
        NifValue::Struct(fields) => fields,
        _ => panic!("not a struct"),
    };
    assert!(matches!(first.get("Start Index"), Some(NifValue::UInt(0))));
    assert!(matches!(
        first.get("Num Primitives"),
        Some(NifValue::UInt(100))
    ));
    assert!(matches!(first.get("User Index"), Some(NifValue::UInt(32))));
    assert!(total_segment_data > 0);
}

#[test]
fn build_segment_data_with_no_specs_emits_single_default_segment() {
    let (num_segments, _, _) = build_segment_data(&[]);
    assert_eq!(num_segments, 1);
}

#[test]
fn convert_legacy_skin_promotes_shape_and_drops_legacy_skin_chain() {
    let (mut nif, _) = make_convertible_legacy_skin_nif();
    let dir = translation_maps_dir();

    let report = convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    assert_eq!(report.shapes_skinned, 1);
    assert_eq!(report.vertices_repacked, 3);
    assert!(
        nif.blocks
            .iter()
            .any(|block| block.type_name == "BSSubIndexTriShape")
    );
    assert!(
        nif.blocks
            .iter()
            .any(|block| block.type_name == "BSSkin::Instance")
    );
    assert!(nif.blocks.iter().all(|block| !matches!(
        block.type_name.as_str(),
        "NiSkinInstance" | "NiSkinData" | "NiSkinPartition"
    )));
}

#[test]
fn convert_legacy_skin_preserves_source_skeleton_root_ref() {
    let (mut nif, shape_id) = make_convertible_legacy_skin_nif();
    let instance_id = skin_instance_id(&nif, shape_id);
    let source_root = match nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Bones"))
    {
        Some(NifValue::Array(bones)) => match bones.first() {
            Some(NifValue::Ref(reference)) if *reference >= 0 => *reference,
            other => panic!("unexpected root bone ref: {other:?}"),
        },
        other => panic!("unexpected bones array: {other:?}"),
    };
    assert_ne!(source_root, 0);
    nif.blocks[instance_id].set_field("Skeleton Root", NifValue::Ref(source_root));

    let dir = translation_maps_dir();
    convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    let shape = nif
        .blocks
        .iter()
        .find(|block| block.type_name == "BSSubIndexTriShape")
        .expect("converted shape");
    let skin_id = match shape.get_field("Skin") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("unexpected converted skin ref: {other:?}"),
    };
    let skeleton_root = match nif
        .get_block(skin_id)
        .and_then(|skin| skin.get_field("Skeleton Root"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference,
        other => panic!("unexpected converted skeleton root: {other:?}"),
    };

    assert_ne!(skeleton_root, 0);
    let root = nif.get_block(skeleton_root as usize).expect("root block");
    assert_eq!(root.type_name, "NiNode");
}

#[test]
fn convert_legacy_skin_handles_shared_bones_across_multiple_shapes() {
    let (mut nif, first_shape_id) = make_convertible_legacy_skin_nif();
    add_convertible_shape_sharing_skin_bones(&mut nif, first_shape_id);
    let dir = translation_maps_dir();

    let report = convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    assert_eq!(report.shapes_skinned, 2);
    let converted_shapes = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
        .collect::<Vec<_>>();
    assert_eq!(converted_shapes.len(), 2);
    assert!(converted_shapes.iter().all(|shape| {
        matches!(shape.get_field("Skin"), Some(NifValue::Ref(reference)) if *reference >= 0)
    }));
}

#[test]
fn convert_legacy_skin_all_unmapped_static_fallback_uses_static_vertex_layout() {
    let (mut nif, shape_id) = make_convertible_legacy_skin_nif();
    let instance_id = skin_instance_id(&nif, shape_id);
    let bone_ids = match nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Bones"))
    {
        Some(NifValue::Array(bones)) => bones
            .iter()
            .filter_map(|value| match value {
                NifValue::Ref(reference) if *reference >= 0 => Some(*reference as usize),
                _ => None,
            })
            .collect::<Vec<_>>(),
        other => panic!("unexpected bones array: {other:?}"),
    };
    for (index, bone_id) in bone_ids.into_iter().enumerate() {
        nif.blocks[bone_id].set_field("Name", NifValue::String(format!("BogusBone{index}")));
    }

    let dir = translation_maps_dir();
    let report = convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    assert_eq!(report.shapes_skinned, 0);
    assert_eq!(report.vertices_repacked, 3);
    assert!(
        nif.blocks
            .iter()
            .all(|block| block.type_name != "BSSkin::Instance")
    );

    let shape = nif
        .blocks
        .iter()
        .find(|block| block.type_name == "BSSubIndexTriShape")
        .expect("converted static shape");
    assert!(matches!(shape.get_field("Skin"), Some(NifValue::Ref(-1))));
    assert!(matches!(
        shape.get_field("Skin Instance"),
        Some(NifValue::Ref(-1))
    ));
    let vertex_desc = shape
        .get_field("Vertex Desc")
        .map(NifValue::as_i64)
        .expect("vertex desc");
    assert_eq!((vertex_desc >> 44) & 0x0040, 0);
    assert_eq!(vertex_desc & 0xF, 5);

    let vertices = match shape.get_field("Vertex Data") {
        Some(NifValue::Array(vertices)) => vertices,
        other => panic!("unexpected vertex data: {other:?}"),
    };
    let first = match vertices.first() {
        Some(NifValue::Struct(fields)) => fields,
        other => panic!("unexpected vertex: {other:?}"),
    };
    assert!(!first.contains_key("Bone Indices"));
    assert!(!first.contains_key("Bone Weights"));
}

#[test]
fn convert_legacy_skin_redistributes_unmapped_child_weight_to_parsed_parent() {
    let (mut nif, shape_id) = make_convertible_legacy_skin_nif();
    let instance_id = skin_instance_id(&nif, shape_id);
    let child_bone_id = match nif
        .get_block(instance_id)
        .and_then(|instance| instance.get_field("Bones"))
    {
        Some(NifValue::Array(bones)) => match bones.get(1) {
            Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
            other => panic!("unexpected child bone ref: {other:?}"),
        },
        other => panic!("unexpected bones array: {other:?}"),
    };
    nif.blocks[child_bone_id].set_field("Name", NifValue::String("Bip01 BogusBone".to_string()));

    let dir = translation_maps_dir();
    let report = convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    assert_eq!(report.shapes_skinned, 1);
    assert_eq!(report.bones_dropped_unmapped, 1);
    assert_eq!(report.weights_redistributed, 2);
}

#[test]
fn convert_legacy_skin_is_atomic_on_later_parse_error() {
    let (mut nif, shape_id) = make_convertible_legacy_skin_nif();
    let invalid_instance_id = nif.add_block(
        "NiSkinInstance",
        Some(fields([
            ("Skin Partition", NifValue::Ref(-1)),
            ("Skeleton Root", NifValue::Ref(0)),
            ("Num Bones", NifValue::UInt(0)),
            ("Bones", NifValue::Array(Vec::new())),
        ])),
    );
    nif.add_block(
        "NiTriShape",
        Some(fields([(
            "Skin Instance",
            NifValue::Ref(invalid_instance_id as i32),
        )])),
    );
    let original_block_count = nif.blocks.len();

    let dir = translation_maps_dir();
    let result = convert_legacy_skin(&mut nif, &dir, None, 0.5);

    assert!(result.is_err());
    assert_eq!(nif.blocks.len(), original_block_count);
    assert_eq!(nif.get_block(shape_id).unwrap().type_name, "NiTriShape");
    assert!(
        nif.blocks
            .iter()
            .all(|block| block.type_name != "BSSkin::Instance")
    );
}

#[test]
fn convert_legacy_skin_preserves_partition_segments() {
    let (mut nif, shape_id) = make_minimal_skinned_nif();
    let instance_id = skin_instance_id(&nif, shape_id);
    nif.blocks[instance_id].type_name = "BSDismemberSkinInstance".to_string();

    if let Some(shape) = nif.blocks.get_mut(shape_id) {
        shape.set_field(
            "Vertex Data",
            NifValue::Array(vec![
                vertex_entry([0.0, 0.0, 0.0]),
                vertex_entry([1.0, 0.0, 0.0]),
                vertex_entry([0.0, 1.0, 0.0]),
                vertex_entry([1.0, 1.0, 0.0]),
            ]),
        );
        shape.set_field(
            "Triangles",
            NifValue::Array(vec![triangle_value([0, 1, 2]), triangle_value([1, 3, 2])]),
        );
        shape.set_field("Num Vertices", NifValue::UInt(4));
        shape.set_field("Num Triangles", NifValue::UInt(2));
    }

    let partition = |body_part: u16, vertex_map: Vec<u32>| {
        let bone_rows = (0..vertex_map.len())
            .map(|_| {
                NifValue::Array(vec![
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                    NifValue::UInt(0),
                ])
            })
            .collect::<Vec<_>>();
        let weight_rows = (0..vertex_map.len())
            .map(|_| {
                NifValue::Array(vec![
                    NifValue::Float(1.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                    NifValue::Float(0.0),
                ])
            })
            .collect::<Vec<_>>();
        NifValue::Struct(fields([
            ("Body Part", NifValue::UInt(body_part as u64)),
            (
                "Vertex Map",
                NifValue::Array(
                    vertex_map
                        .into_iter()
                        .map(|index| NifValue::UInt(index as u64))
                        .collect(),
                ),
            ),
            ("Bone Indices", NifValue::Array(bone_rows)),
            ("Vertex Weights", NifValue::Array(weight_rows)),
            ("Bones", NifValue::Array(vec![NifValue::UInt(0)])),
            (
                "Triangles",
                NifValue::Array(vec![triangle_value([0, 1, 2])]),
            ),
        ]))
    };

    let partition_id = skin_partition_id(&nif, shape_id);
    let partition_block = nif.blocks.get_mut(partition_id).expect("partition block");
    partition_block.set_field("Num Partitions", NifValue::UInt(2));
    partition_block.set_field(
        "Partitions",
        NifValue::Array(vec![
            partition(0, vec![0, 1, 2]),
            partition(2, vec![1, 3, 2]),
        ]),
    );

    let dir = translation_maps_dir();
    let report = convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    assert_eq!(report.shapes_skinned, 1);
    let shape = nif
        .blocks
        .iter()
        .find(|block| block.type_name == "BSSubIndexTriShape")
        .expect("converted shape");
    assert_eq!(
        shape.get_field("Num Segments").map(NifValue::as_i64),
        Some(2)
    );
    let segments = match shape.get_field("Segment") {
        Some(NifValue::Array(segments)) => segments,
        other => panic!("expected segment array, got {other:?}"),
    };
    let user_indices = segments
        .iter()
        .filter_map(|value| match value {
            NifValue::Struct(fields) => fields.get("User Index").map(NifValue::as_i64),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(user_indices, vec![32, 34]);
}

#[test]
fn convert_legacy_skin_uses_skin_data_fallback_and_bind_transforms() {
    let (mut nif, shape_id) = make_minimal_skinned_nif();
    if let Some(shape) = nif.blocks.get_mut(shape_id) {
        shape.set_field(
            "Vertex Data",
            NifValue::Array(vec![
                vertex_entry([0.0, 0.0, 0.0]),
                vertex_entry([1.0, 0.0, 0.0]),
                vertex_entry([0.0, 1.0, 0.0]),
            ]),
        );
        shape.set_field(
            "Triangles",
            NifValue::Array(vec![triangle_value([0, 1, 2])]),
        );
        shape.set_field("Num Vertices", NifValue::UInt(3));
        shape.set_field("Num Triangles", NifValue::UInt(1));
    }
    let skin_data_id = skin_data_id(&nif, shape_id);
    let skin_data = nif.blocks.get_mut(skin_data_id).expect("skin data");
    skin_data.set_field("Skin Transform", skin_transform_value([10.0, 0.0, 0.0]));
    skin_data.set_field(
        "Bone List",
        NifValue::Array(vec![
            bone_data_value([0.0, 0.0, 0.0], vec![bone_weight_value(0, 1.0)]),
            bone_data_value(
                [0.0, 5.0, 0.0],
                vec![bone_weight_value(1, 1.0), bone_weight_value(2, 1.0)],
            ),
        ]),
    );

    let dir = translation_maps_dir();
    let report = convert_legacy_skin(&mut nif, &dir, None, 0.5).expect("convert skin");

    assert_eq!(report.shapes_skinned, 1);
    let shape = nif
        .blocks
        .iter()
        .find(|block| block.type_name == "BSSubIndexTriShape")
        .expect("converted shape");
    let vertices = match shape.get_field("Vertex Data") {
        Some(NifValue::Array(vertices)) => vertices,
        other => panic!("expected vertex data, got {other:?}"),
    };
    let second_vertex = match &vertices[1] {
        NifValue::Struct(fields) => fields,
        other => panic!("expected vertex struct, got {other:?}"),
    };
    assert_eq!(
        second_vertex.get("Vertex").and_then(|value| match value {
            NifValue::Vec3(position) => Some(*position),
            _ => None,
        }),
        Some([11.0, 0.0, 0.0])
    );
    assert!(matches!(
        second_vertex.get("Bone Indices"),
        Some(NifValue::Array(indices)) if indices.first().map(NifValue::as_i64) == Some(1)
    ));

    let skin_id = match shape.get_field("Skin") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("expected skin ref, got {other:?}"),
    };
    let bone_data_id = match nif
        .get_block(skin_id)
        .and_then(|skin| skin.get_field("Data"))
    {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("expected bone data ref, got {other:?}"),
    };
    let bone_list = match nif
        .get_block(bone_data_id)
        .and_then(|bone_data| bone_data.get_field("Bone List"))
    {
        Some(NifValue::Array(bone_list)) => bone_list,
        other => panic!("expected bone list, got {other:?}"),
    };
    let second_bone = match &bone_list[1] {
        NifValue::Struct(fields) => fields,
        other => panic!("expected bone data struct, got {other:?}"),
    };
    assert_eq!(
        second_bone
            .get("Translation")
            .and_then(|value| match value {
                NifValue::Vec3(translation) => Some(*translation),
                _ => None,
            }),
        Some([0.0, 5.0, 0.0])
    );
}

#[test]
fn restructure_bone_tree_appends_skin_bones_to_scene_root() {
    let mut nif = NifFile::new("fo4");
    let bone_id = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("Pelvis".to_string()))])),
    );
    nif.add_block(
        "BSSkin::Instance",
        Some(fields([
            ("Skeleton Root", NifValue::Ref(0)),
            ("Data", NifValue::Ref(-1)),
            ("Num Bones", NifValue::UInt(1)),
            (
                "Bones",
                NifValue::Array(vec![NifValue::Ref(bone_id as i32)]),
            ),
        ])),
    );

    restructure_bone_tree(&mut nif);

    let children = match nif.blocks[0].get_field("Children") {
        Some(NifValue::Array(children)) => children,
        other => panic!("expected children array, got {other:?}"),
    };
    assert!(
        children
            .iter()
            .any(|value| matches!(value, NifValue::Ref(id) if *id == bone_id as i32))
    );
}

#[test]
fn extract_arm_subset_keeps_arm_dominant_triangles() {
    let positions = vec![
        [0.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    let influences = vec![
        VertexInfluences {
            slots: vec![(1, 1.0)],
        },
        VertexInfluences {
            slots: vec![(1, 1.0)],
        },
        VertexInfluences {
            slots: vec![(1, 1.0)],
        },
        VertexInfluences {
            slots: vec![(0, 1.0)],
        },
    ];
    let triangles = vec![[0, 1, 2], [1, 2, 3]];
    let bone_names = vec!["Pelvis".to_string(), "LArm_ForeArm1".to_string()];

    let extract =
        extract_arm_subset(&positions, &influences, &triangles, &bone_names).expect("arm subset");

    assert_eq!(extract.kept_triangles.len(), 2);
    assert_eq!(extract.kept_positions.len(), 4);
    assert_eq!(extract.arm_bones_used, vec![1]);
}

#[test]
fn extract_arm_subset_returns_none_without_arm_weights() {
    let positions = vec![[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let influences = vec![
        VertexInfluences {
            slots: vec![(0, 1.0)],
        },
        VertexInfluences {
            slots: vec![(0, 1.0)],
        },
        VertexInfluences {
            slots: vec![(0, 1.0)],
        },
    ];
    let triangles = vec![[0, 1, 2]];
    let bone_names = vec!["Pelvis".to_string(), "Spine1".to_string()];

    assert!(extract_arm_subset(&positions, &influences, &triangles, &bone_names).is_none());
}

#[test]
fn transfer_morph_weights_adds_reference_only_bone_with_cap() {
    let target_positions = vec![[0.0_f32, 0.0, 0.0]];
    let mut target_influences = vec![VertexInfluences {
        slots: vec![(0, 1.0)],
    }];
    let mut target_bones = vec!["Pelvis".to_string()];
    let ref_positions = vec![[0.0_f32, 0.0, 0.0]];
    let ref_influences = vec![VertexInfluences {
        slots: vec![(0, 0.5), (1, 0.5)],
    }];
    let ref_bones = vec!["Pelvis".to_string(), "BodyMorph".to_string()];

    let stats = transfer_morph_weights(
        &target_positions,
        &mut target_influences,
        &mut target_bones,
        &ref_positions,
        &ref_influences,
        &ref_bones,
        &MorphTransferConfig {
            morph_weight_cap: 0.25,
            k_neighbors: 1,
        },
    );

    assert_eq!(stats.vertices_morph_weighted, 1);
    assert_eq!(stats.morph_bones_added, vec!["BodyMorph"]);
    assert_eq!(target_bones, vec!["Pelvis", "BodyMorph"]);
    let morph_weight = target_influences[0]
        .slots
        .iter()
        .find_map(|(bone, weight)| (*bone == 1).then_some(*weight))
        .expect("morph weight");
    assert!(morph_weight <= 0.2501, "{morph_weight}");
}

#[test]
fn transfer_morph_weights_enforces_cap_after_top_four_normalization() {
    let target_positions = vec![[0.0_f32, 0.0, 0.0]];
    let mut target_influences = vec![VertexInfluences {
        slots: vec![(0, 0.25), (1, 0.25), (2, 0.25), (3, 0.25)],
    }];
    let mut target_bones = vec![
        "Pelvis".to_string(),
        "Spine1".to_string(),
        "Spine2".to_string(),
        "Arm".to_string(),
    ];
    let ref_positions = vec![[0.0_f32, 0.0, 0.0]];
    let ref_influences = vec![VertexInfluences {
        slots: vec![(4, 1.0)],
    }];
    let ref_bones = vec![
        "Pelvis".to_string(),
        "Spine1".to_string(),
        "Spine2".to_string(),
        "Arm".to_string(),
        "BodyMorph".to_string(),
    ];

    transfer_morph_weights(
        &target_positions,
        &mut target_influences,
        &mut target_bones,
        &ref_positions,
        &ref_influences,
        &ref_bones,
        &MorphTransferConfig {
            morph_weight_cap: 0.25,
            k_neighbors: 1,
        },
    );

    let morph_weight = target_influences[0]
        .slots
        .iter()
        .find_map(|(bone, weight)| (*bone == 4).then_some(*weight))
        .expect("morph weight");
    let total: f32 = target_influences[0]
        .slots
        .iter()
        .map(|(_, weight)| *weight)
        .sum();
    assert!(morph_weight <= 0.2501, "{morph_weight}");
    assert!((total - 1.0).abs() < 1e-5, "{total}");
}

fn vertex_entry(position: [f32; 3]) -> NifValue {
    NifValue::Struct(fields([
        ("Vertex", NifValue::Vec3(position)),
        ("Bitangent X", NifValue::Float(0.0)),
        ("UV", tex_coord([0.0, 0.0])),
        ("Normal", NifValue::Vec3([0.0, 0.0, 1.0])),
        ("Bitangent Y", NifValue::Float(1.0)),
        ("Tangent", NifValue::Vec3([1.0, 0.0, 0.0])),
        ("Bitangent Z", NifValue::Float(0.0)),
    ]))
}

fn triangle_value(triangle: [u32; 3]) -> NifValue {
    NifValue::Struct(fields([
        ("v1", NifValue::UInt(triangle[0] as u64)),
        ("v2", NifValue::UInt(triangle[1] as u64)),
        ("v3", NifValue::UInt(triangle[2] as u64)),
    ]))
}

fn tex_coord(uv: [f32; 2]) -> NifValue {
    NifValue::Struct(fields([
        ("u", NifValue::Float(uv[0] as f64)),
        ("v", NifValue::Float(uv[1] as f64)),
    ]))
}

fn fields<const N: usize>(entries: [(&str, NifValue); N]) -> IndexMap<String, NifValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}
