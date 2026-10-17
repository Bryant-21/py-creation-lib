use super::*;

fn fields<const N: usize>(entries: [(&str, NifValue); N]) -> IndexMap<String, NifValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect()
}

fn transform(translation: [f32; 3], rotation: [[f32; 3]; 3], scale: f32) -> NifValue {
    NifValue::Struct(fields([
        ("Translation", NifValue::Vec3(translation)),
        ("Rotation", NifValue::Matrix33(rotation)),
        ("Scale", NifValue::Float(scale.into())),
    ]))
}

pub(super) fn source_fixture(global_to_skin: NifValue) -> (NifFile, usize) {
    let mut nif = NifFile::new("fnv");
    let identity = SkinTransform::identity().rotation;
    let root = nif.add_block(
        "NiNode",
        Some(fields([
            ("Name", NifValue::String("Root".into())),
            ("Num Children", NifValue::UInt(1)),
            ("Children", NifValue::Array(vec![NifValue::Ref(1)])),
        ])),
    );
    let child = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("Child".into()))])),
    );
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let data = nif.add_block(
        "NiTriShapeData",
        Some(fields([
            ("Num Vertices", NifValue::UInt(3)),
            (
                "Vertices",
                NifValue::Array(positions.into_iter().map(NifValue::Vec3).collect()),
            ),
            (
                "Normals",
                NifValue::Array(vec![NifValue::Vec3([1.0, 0.0, 0.0]); 3]),
            ),
            (
                "Triangles",
                NifValue::Array(vec![triangle_value([0, 1, 2])]),
            ),
        ])),
    );
    let bones = [(root, vec![0]), (child, vec![1, 2])];
    let bone_data = bones
        .iter()
        .map(|(_, vertices)| {
            NifValue::Struct(fields([
                ("Skin Transform", transform([0.0; 3], identity, 1.0)),
                (
                    "Bounding Sphere",
                    NifValue::Struct(fields([
                        ("Center", NifValue::Vec3([0.0; 3])),
                        ("Radius", NifValue::Float(2.0)),
                    ])),
                ),
                (
                    "Vertex Weights",
                    NifValue::Array(
                        vertices
                            .iter()
                            .map(|vertex| {
                                NifValue::Struct(fields([
                                    ("Index", NifValue::UInt(*vertex)),
                                    ("Weight", NifValue::Float(1.0)),
                                ]))
                            })
                            .collect(),
                    ),
                ),
            ]))
        })
        .collect();
    let skin_data = nif.add_block(
        "NiSkinData",
        Some(fields([
            ("Skin Transform", global_to_skin),
            ("Num Bones", NifValue::UInt(2)),
            ("Bone List", NifValue::Array(bone_data)),
        ])),
    );
    let partition = nif.add_block(
        "NiSkinPartition",
        Some(fields([
            ("Num Partitions", NifValue::UInt(1)),
            (
                "Partitions",
                NifValue::Array(vec![NifValue::Struct(fields([
                    (
                        "Vertex Map",
                        NifValue::Array((0..3).map(NifValue::UInt).collect()),
                    ),
                    (
                        "Bones",
                        NifValue::Array(vec![NifValue::UInt(0), NifValue::UInt(1)]),
                    ),
                    (
                        "Bone Indices",
                        NifValue::Array(
                            [0, 1, 1]
                                .into_iter()
                                .map(|bone| {
                                    NifValue::Array(vec![
                                        NifValue::UInt(bone),
                                        NifValue::UInt(0),
                                        NifValue::UInt(0),
                                        NifValue::UInt(0),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "Vertex Weights",
                        NifValue::Array(vec![
                            NifValue::Array(
                                [1.0, 0.0, 0.0, 0.0]
                                    .into_iter()
                                    .map(NifValue::Float)
                                    .collect()
                            );
                            3
                        ]),
                    ),
                    (
                        "Triangles",
                        NifValue::Array(vec![triangle_value([0, 1, 2])]),
                    ),
                ]))]),
            ),
        ])),
    );
    let skin = nif.add_block(
        "NiSkinInstance",
        Some(fields([
            ("Skeleton Root", NifValue::Ref(root as i32)),
            ("Data", NifValue::Ref(skin_data as i32)),
            ("Skin Partition", NifValue::Ref(partition as i32)),
            ("Num Bones", NifValue::UInt(2)),
            (
                "Bones",
                NifValue::Array(
                    bones
                        .iter()
                        .map(|(bone, _)| NifValue::Ref(*bone as i32))
                        .collect(),
                ),
            ),
        ])),
    );
    let shape = nif.add_block(
        "NiTriShape",
        Some(fields([
            ("Name", NifValue::String("Body".into())),
            ("Data", NifValue::Ref(data as i32)),
            ("Skin Instance", NifValue::Ref(skin as i32)),
        ])),
    );
    (nif, shape)
}

#[test]
fn preserved_rig_keeps_skin_space_under_translation_rotation_and_scale() {
    let identity = SkinTransform::identity().rotation;
    let turn = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    for source_game in ["fnv", "fo3", "skyrimse"] {
        for global_to_skin in [
            transform([0.0; 3], identity, 1.0),
            transform([10.0, -20.0, 5.0], identity, 1.0),
            transform([10.0, -20.0, 5.0], turn, 2.0),
        ] {
            let (mut nif, _) = source_fixture(global_to_skin);
            convert_legacy_skin_for_games_with_policy(
                &mut nif,
                None,
                source_game,
                "fo4",
                None,
                0.0,
                LegacySkinPolicy::PreserveSourceRig,
            )
            .unwrap();
            let shape = nif.blocks.iter().find(|block| {
                matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Body")
            }).unwrap();
            let geometry = ShapeGeometry::from_shape(&nif, shape);
            let influences = packed_shape_influences(shape, geometry.positions.len());
            let posed = geometry
                .positions
                .iter()
                .zip(influences)
                .map(|(position, influence)| {
                    let mut point = [0.0; 3];
                    for (bone, weight) in influence.slots {
                        let moved = if bone == 0 {
                            *position
                        } else {
                            [3.0 - position[1], 2.0 + position[0], position[2]]
                        };
                        for axis in 0..3 {
                            point[axis] += weight * moved[axis];
                        }
                    }
                    point
                })
                .collect::<Vec<_>>();
            assert_eq!(
                posed,
                [[0.0, 0.0, 0.0], [3.0, 3.0, 0.0], [2.0, 2.0, 0.0]],
                "{source_game}"
            );
            assert_eq!(geometry.normals, vec![[1.0, 0.0, 0.0]; 3]);
        }
    }
}

fn packed_shape_influences(shape: &NifBlock, count: usize) -> Vec<VertexInfluences> {
    let NifValue::Array(vertices) = shape.get_field("Vertex Data").unwrap() else {
        panic!("vertex data")
    };
    assert_eq!(vertices.len(), count);
    vertices
        .iter()
        .map(|vertex| {
            let NifValue::Struct(fields) = vertex else {
                panic!("vertex")
            };
            let indices = numeric_array(fields.get("Bone Indices"));
            let weights = float_array(fields.get("Bone Weights"));
            VertexInfluences {
                slots: indices
                    .into_iter()
                    .zip(weights)
                    .filter_map(|(bone, weight)| (weight > 0.0).then_some((bone as usize, weight)))
                    .collect(),
            }
        })
        .collect()
}

#[test]
#[ignore = "requires the CLI-derived B21_CREATURE_SKIN_CORPUS manifest and extracted game assets"]
fn real_creature_corpus_preserves_skin_space_after_save_reload() {
    let manifest = std::path::PathBuf::from(
        std::env::var("B21_CREATURE_SKIN_CORPUS").expect("corpus manifest"),
    );
    let cases: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert!(!cases.is_empty());
    let temp = tempfile::tempdir().unwrap();
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let stage = temp.path().join(index.to_string());
        match audit_saved_skin(case, &stage) {
            Ok(result) => results.push(result),
            Err(error) => failures
                .push(serde_json::json!({"source_path": case["source_path"], "error": error})),
        }
    }
    let report = serde_json::json!({"models": cases.len(), "passed": results.len(), "failures": failures, "results": results});
    let output = manifest.with_file_name("creature_skin_conversion_audit.json");
    std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!(
        "creature skin corpus: {} models, {} passed, {} failures; {}",
        cases.len(),
        results.len(),
        failures.len(),
        output.display()
    );
    assert!(
        failures.is_empty(),
        "{}",
        serde_json::to_string_pretty(&failures).unwrap()
    );
}

pub(super) fn audit_saved_skin(
    case: &serde_json::Value,
    stage: &Path,
) -> Result<serde_json::Value, String> {
    let source_path = Path::new(case["source_path"].as_str().ok_or("source_path")?);
    let source_game = case["source_game"].as_str().ok_or("source_game")?;
    let source = NifFile::load(source_path).map_err(|error| error.to_string())?;
    let mut expected = Vec::new();
    for shape in &source.blocks {
        let Some(skin) =
            source::parse_skin_chain(&source, shape.block_id).map_err(|error| error.to_string())?
        else {
            continue;
        };
        let mut geometry = ShapeGeometry::from_shape(&source, shape);
        geometry.complete_partition_geometry(&source, shape, &skin);
        expected.push((shape.get_field("Name").cloned(), geometry.positions, skin));
    }
    std::fs::create_dir_all(stage).map_err(|error| error.to_string())?;
    let output = stage.join("body.nif");
    crate::convert_file::stage_preserve_source_rig_nif(
        if case["body_part_replacement"].as_bool() == Some(true) {
            crate::convert_file::SourceRigNifKind::BodyPartReplacement
        } else {
            crate::convert_file::SourceRigNifKind::Body
        },
        source_path,
        &output,
        "Meshes/CreatureAudit/body.nif",
        source_game,
        Some(&stage.join("materials")),
        &crate::convert_file::ConvertFileOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let target = NifFile::load(&output).map_err(|error| error.to_string())?;
    if case["body_part_replacement"].as_bool() == Some(true) {
        let source_collision = source.blocks.iter().any(|block| {
            matches!(
                block.type_name.as_str(),
                "bhkCollisionObject" | "bhkBlendCollisionObject"
            )
        });
        if source_collision
            && (!target
                .blocks
                .iter()
                .any(|block| block.type_name == "bhkNPCollisionObject")
                || !target.blocks.iter().any(|block| {
                    matches!(
                        block.type_name.as_str(),
                        "bhkPhysicsSystem" | "bhkRagdollSystem"
                    )
                }))
        {
            return Err("replacement lost source embedded collision".into());
        }
        if target.blocks.iter().any(|block| {
            matches!(
                block.type_name.as_str(),
                "bhkCollisionObject" | "bhkBlendCollisionObject" | "bhkRigidBody" | "bhkRigidBodyT"
            )
        }) {
            return Err("replacement retained source collision blocks".into());
        }
    }
    let shapes = target
        .blocks
        .iter()
        .filter(|shape| {
            value_ref(shape.get_field("Skin"))
                .filter(|id| *id >= 0)
                .and_then(|id| target.get_block(id as usize))
                .is_some_and(|skin| skin.type_name == "BSSkin::Instance")
        })
        .collect::<Vec<_>>();
    if shapes.len() != expected.len() {
        return Err(format!("skin count {} -> {}", expected.len(), shapes.len()));
    }
    let mut vertex_count = 0;
    let mut max_position_error = 0.0_f32;
    let mut nonidentity_skin_transforms = 0;
    for ((name, positions, source_skin), shape) in expected.into_iter().zip(shapes) {
        if name.as_ref() != shape.get_field("Name") {
            return Err("skin shape order/name changed".into());
        }
        let geometry = ShapeGeometry::from_shape(&target, shape);
        if positions.len() != geometry.positions.len() {
            return Err("vertex count changed".into());
        }
        nonidentity_skin_transforms += usize::from(!source_skin.skin_transform.is_identity());
        vertex_count += positions.len();
        for (source, target) in positions.iter().zip(&geometry.positions) {
            for axis in 0..3 {
                let error = (source[axis] - target[axis]).abs();
                if !target[axis].is_finite() || error > source[axis].abs() * 0.0006 + 0.001 {
                    return Err(format!(
                        "{name:?}: skin-space vertex changed {source:?} -> {target:?}"
                    ));
                }
                max_position_error = max_position_error.max(error);
            }
        }
        let instance = target
            .get_block(value_ref(shape.get_field("Skin")).unwrap() as usize)
            .ok_or("skin instance")?;
        let bone_data = target
            .get_block(value_ref(instance.get_field("Data")).ok_or("bone data ref")? as usize)
            .ok_or("bone data")?;
        let palette = ref_array(instance.get_field("Bones"));
        let binds = value_array(bone_data.get_field("Bone List"));
        if palette.len() != source_skin.bones.len() || binds.len() != palette.len() {
            return Err("bone palette count changed".into());
        }
        for (index, (bone, bind)) in palette.iter().zip(&binds).enumerate() {
            let node = target.get_block(*bone as usize).ok_or("palette node")?;
            if !matches!(node.get_field("Name"), Some(NifValue::String(name)) if name.trim_end_matches('\0') == source_skin.bones[index].name)
            {
                return Err(format!("palette bone {index} changed"));
            }
            let NifValue::Struct(bind) = bind else {
                return Err("bind transform".into());
            };
            let target_bind = SkinTransform {
                translation: bind
                    .get("Translation")
                    .and_then(vec3_value)
                    .ok_or("bind translation")?,
                rotation: bind
                    .get("Rotation")
                    .and_then(source::matrix33_value)
                    .ok_or("bind rotation")?,
                scale: value_f64(bind.get("Scale")).ok_or("bind scale")? as f32,
            };
            if target_bind != source_skin.bone_transforms[index] {
                return Err(format!("{name:?}: bind {index} changed"));
            }
        }
        let original =
            source_rig_influences(&source_skin, positions.len()).ok_or("source weights")?;
        let converted = packed_shape_influences(shape, positions.len());
        for (before, after) in original.iter().zip(&converted) {
            for (bone, weight) in &before.slots {
                if *weight == 0.0 {
                    continue;
                }
                let target_weight = after
                    .slots
                    .iter()
                    .filter(|(index, _)| index == bone)
                    .map(|(_, weight)| *weight)
                    .sum::<f32>();
                let source_weight = before
                    .slots
                    .iter()
                    .filter(|(index, _)| index == bone)
                    .map(|(_, weight)| *weight)
                    .sum::<f32>();
                if (target_weight - source_weight).abs() > 0.001 {
                    return Err("skin weights changed".into());
                }
            }
        }
    }
    Ok(serde_json::json!({
        "source_game": source_game, "source_path": source_path, "family_id": case["family_id"],
        "vertices": vertex_count, "nonidentity_skin_transforms": nonidentity_skin_transforms,
        "max_position_quantization_error": max_position_error,
    }))
}
