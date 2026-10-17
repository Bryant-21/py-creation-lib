use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::segment::SegmentSpec;
use super::source::{LegacySkinKind, parse_skin_chain};
use super::{ShapeGeometry, source_rig_influences, triangle_value, write_segment_specs};
use crate::model::{NifFile, NifValue};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatureBodyPartSegment {
    pub node: String,
    pub index: u8,
    pub source_partition: Option<u16>,
}

pub fn segment_source_creature_body(
    source: &NifFile,
    converted: &mut NifFile,
    segments: &[CreatureBodyPartSegment],
) -> Result<usize, String> {
    if segments.is_empty() {
        return Ok(0);
    }
    let mut indices = BTreeSet::new();
    for segment in segments {
        if segment.index == 32 || !indices.insert(segment.index) || segment.node.is_empty() {
            return Err("invalid creature body-part segment plan".to_string());
        }
    }
    let mut source_shapes = BTreeMap::<String, Vec<usize>>::new();
    for shape in &source.blocks {
        if parse_skin_chain(source, shape.block_id)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            let Some(NifValue::String(name)) = shape.get_field("Name") else {
                return Err("source skinned shape has no name".to_string());
            };
            source_shapes
                .entry(name.clone())
                .or_default()
                .push(shape.block_id);
        }
    }
    let mut matches = HashMap::<String, usize>::new();
    let mut count = 0;
    for shape in &mut converted.blocks {
        if shape.type_name != "BSSubIndexTriShape"
            || !matches!(shape.get_field("Skin"), Some(NifValue::Ref(reference)) if *reference >= 0)
        {
            continue;
        }
        let Some(NifValue::String(name)) = shape.get_field("Name") else {
            continue;
        };
        let ordinal = matches.entry(name.clone()).or_default();
        let Some(source_shape_id) = source_shapes.get(name).and_then(|ids| ids.get(*ordinal))
        else {
            continue;
        };
        *ordinal += 1;
        let source_shape = source
            .get_block(*source_shape_id)
            .ok_or("source shape missing")?;
        let skin = parse_skin_chain(source, *source_shape_id)
            .map_err(|error| error.to_string())?
            .ok_or("source skin missing")?;
        let mut geometry = ShapeGeometry::from_shape(source, source_shape);
        geometry.complete_partition_geometry(source, source_shape, &skin);
        let mut original_triangles = geometry.triangles.clone();
        original_triangles.sort_unstable();
        let Some(NifValue::Array(converted_triangles)) = shape.get_field("Triangles") else {
            return Err("converted creature shape has no triangles".into());
        };
        let mut converted_triangles = converted_triangles
            .iter()
            .filter_map(super::triangle_from_value)
            .collect::<Vec<_>>();
        converted_triangles.sort_unstable();
        if original_triangles != converted_triangles {
            return Err("creature conversion changed triangle topology before segmentation".into());
        }
        let influences = source_rig_influences(&skin, geometry.positions.len())
            .ok_or("source body-part weights are invalid")?;
        let parents = source
            .blocks
            .iter()
            .flat_map(|block| match block.get_field("Children") {
                Some(NifValue::Array(children)) => children
                    .iter()
                    .filter_map(|value| match value {
                        NifValue::Ref(child) if *child >= 0 => {
                            Some((*child as usize, block.block_id))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect::<HashMap<_, _>>();
        let nearest = skin.bones.iter().map(|bone| {
            let mut current = source.blocks.iter().find(|block| matches!(block.get_field("Name"), Some(NifValue::String(name)) if name.eq_ignore_ascii_case(&bone.name))).map(|block| block.block_id);
            let mut visited = BTreeSet::new();
            while let Some(index) = current {
                if !visited.insert(index) { break; }
                let block = source.get_block(index)?;
                let Some(NifValue::String(name)) = block.get_field("Name") else { break };
                if let Some(segment) = segments.iter().find(|part| part.node.eq_ignore_ascii_case(name)) {
                    return Some(segment.index);
                }
                current = parents.get(&index).copied();
            }
            None
        }).collect::<Vec<_>>();
        let mut assignments = vec![None; geometry.triangles.len()];
        if skin.kind == LegacySkinKind::Armor {
            let mut lookup = HashMap::<[u32; 3], Vec<usize>>::new();
            for (index, triangle) in geometry.triangles.iter().enumerate() {
                lookup
                    .entry(super::sorted_triangle(*triangle))
                    .or_default()
                    .push(index);
            }
            let global = super::partition_triangles_use_global_vertices(source, &skin);
            for partition in &skin.partitions {
                let (source_partition, user_index) = match partition.body_part {
                    101..=113 => (partition.body_part - 100, 100),
                    201..=213 => (partition.body_part - 200, 101),
                    value => (value, 32),
                };
                let matches = segments
                    .iter()
                    .filter(|segment| segment.source_partition == Some(source_partition))
                    .collect::<Vec<_>>();
                let [segment] = matches.as_slice() else {
                    continue;
                };
                for triangle in &partition.triangles {
                    let triangle = if global {
                        Some(*triangle)
                    } else {
                        super::partition_triangle_to_global(*triangle, &partition.vertex_map)
                    };
                    if let Some(triangle) = triangle {
                        if let Some(candidates) = lookup.get(&super::sorted_triangle(triangle)) {
                            if let Some(index) = candidates
                                .iter()
                                .find(|index| assignments[**index].is_none())
                            {
                                assignments[*index] = Some((segment.index, user_index));
                            }
                        }
                    }
                }
            }
        }
        let mut groups = BTreeMap::<(u8, u32), Vec<[u32; 3]>>::new();
        for (triangle, assigned) in geometry.triangles.iter().zip(assignments) {
            let mut weights = BTreeMap::<u8, f32>::new();
            if assigned.is_none() {
                for vertex in triangle {
                    let influence = influences
                        .get(*vertex as usize)
                        .ok_or("source triangle vertex is out of range")?;
                    for (bone, weight) in &influence.slots {
                        *weights
                            .entry(nearest.get(*bone).copied().flatten().unwrap_or(32))
                            .or_default() += weight;
                    }
                }
            }
            let index = assigned
                .or_else(|| {
                    weights
                        .into_iter()
                        .max_by(|left, right| {
                            left.1
                                .total_cmp(&right.1)
                                .then_with(|| right.0.cmp(&left.0))
                        })
                        .map(|(index, _)| (index, 32))
                })
                .unwrap_or((32, 32));
            groups.entry(index).or_default().push(*triangle);
        }
        let max_index = segments
            .iter()
            .map(|segment| segment.index)
            .max()
            .unwrap_or(32)
            .max(32);
        let mut triangles = Vec::with_capacity(geometry.triangles.len());
        let mut specs = Vec::new();
        let has_caps = groups.keys().any(|(_, user_index)| *user_index >= 100);
        let mut children = Vec::new();
        for index in 0..=max_index {
            let start = triangles.len();
            let mut child_specs = Vec::new();
            for user_index in [32, 100, 101] {
                let group = groups.remove(&(index, user_index)).unwrap_or_default();
                if !group.is_empty() {
                    child_specs.push(SegmentSpec {
                        triangle_start: triangles.len() as u32,
                        triangle_count: group.len() as u32,
                        user_index,
                    });
                }
                triangles.extend(group);
            }
            specs.push(SegmentSpec {
                triangle_start: start as u32,
                triangle_count: (triangles.len() - start) as u32,
                user_index: u32::from(index),
            });
            children.push(child_specs);
        }
        shape.set_field(
            "Triangles",
            NifValue::Array(triangles.into_iter().map(triangle_value).collect()),
        );
        write_segment_specs(shape, geometry.triangles.len(), &specs);
        if has_caps {
            write_meatcap_subsegments(shape, &children)?;
        }
        count += 1;
    }
    let expected = source_shapes.values().map(Vec::len).sum::<usize>();
    if count != expected {
        return Err(format!(
            "creature segmentation matched {count} of {expected} source skins"
        ));
    }
    Ok(count)
}

fn write_meatcap_subsegments(
    shape: &mut crate::model::NifBlock,
    children: &[Vec<SegmentSpec>],
) -> Result<(), String> {
    use indexmap::IndexMap;
    let Some(NifValue::Array(segments)) = shape.get_field_mut("Segment") else {
        return Err("missing parent segments".into());
    };
    let mut starts = Vec::new();
    let mut shared = Vec::new();
    let shared_entry = |user_index: u32| {
        NifValue::Struct(IndexMap::from([
            ("User Index".into(), NifValue::UInt(u64::from(user_index))),
            ("Bone ID".into(), NifValue::UInt(u64::from(u32::MAX))),
            ("Num Cut Offsets".into(), NifValue::UInt(0)),
            ("Cut Offsets".into(), NifValue::Array(Vec::new())),
        ]))
    };
    for (index, (parent, children)) in segments.iter_mut().zip(children).enumerate() {
        let NifValue::Struct(fields) = parent else {
            return Err("invalid parent segment".into());
        };
        let parent_index = shared.len() as u64;
        starts.push(NifValue::UInt(parent_index));
        shared.push(shared_entry(index as u32));
        let mut parts = Vec::new();
        for child in children {
            parts.push(NifValue::Struct(IndexMap::from([
                (
                    "Start Index".into(),
                    NifValue::UInt(u64::from(child.triangle_start) * 3),
                ),
                (
                    "Num Primitives".into(),
                    NifValue::UInt(u64::from(child.triangle_count)),
                ),
                ("Parent Array Index".into(), NifValue::UInt(parent_index)),
                ("Unused".into(), NifValue::UInt(0)),
            ])));
            shared.push(shared_entry(child.user_index));
        }
        fields.insert(
            "Num Sub Segments".into(),
            NifValue::UInt(parts.len() as u64),
        );
        fields.insert("Sub Segment".into(), NifValue::Array(parts));
    }
    let count = children.len() as u64;
    let total = shared.len() as u64;
    shape.set_field("Total Segments", NifValue::UInt(total));
    shape.set_field(
        "Segment Data",
        NifValue::Struct(IndexMap::from([
            ("Num Segments".into(), NifValue::UInt(count)),
            ("Total Segments".into(), NifValue::UInt(total)),
            ("Segment Starts".into(), NifValue::Array(starts)),
            ("Per Segment Data".into(), NifValue::Array(shared)),
            ("SSF File".into(), NifValue::String(String::new())),
        ])),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin::{LegacySkinPolicy, convert_legacy_skin_for_games_with_policy};

    fn fixture() -> (NifFile, usize) {
        super::super::skin_space_tests::source_fixture(NifValue::Struct(indexmap::IndexMap::from(
            [
                ("Translation".to_string(), NifValue::Vec3([0.0; 3])),
                (
                    "Rotation".to_string(),
                    NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
                ),
                ("Scale".to_string(), NifValue::Float(1.0)),
            ],
        )))
    }

    fn converted(source: &NifFile) -> NifFile {
        let mut target = source.clone();
        convert_legacy_skin_for_games_with_policy(
            &mut target,
            None,
            "fnv",
            "fo4",
            None,
            0.0,
            LegacySkinPolicy::PreserveSourceRig,
        )
        .unwrap();
        target
    }

    fn nonempty_segment(target: &NifFile) -> usize {
        let shape = target
            .blocks
            .iter()
            .find(|block| block.type_name == "BSSubIndexTriShape")
            .unwrap();
        let Some(NifValue::Array(segments)) = shape.get_field("Segment") else {
            panic!("segments")
        };
        segments.iter().position(|value| matches!(value, NifValue::Struct(fields) if fields.get("Num Primitives").unwrap().as_i64() > 0)).unwrap()
    }

    #[test]
    fn source_bone_weights_select_limb_without_rebinding_vertices() {
        let (source, _) = fixture();
        let mut target = converted(&source);
        let before = target
            .blocks
            .iter()
            .find(|block| block.type_name == "BSSubIndexTriShape")
            .unwrap()
            .get_field("Vertex Data")
            .cloned();
        let plan = [CreatureBodyPartSegment {
            node: "Child".into(),
            index: 4,
            source_partition: None,
        }];
        assert_eq!(
            segment_source_creature_body(&source, &mut target, &plan).unwrap(),
            1
        );
        assert_eq!(nonempty_segment(&target), 4);
        assert_eq!(
            before,
            target
                .blocks
                .iter()
                .find(|block| block.type_name == "BSSubIndexTriShape")
                .unwrap()
                .get_field("Vertex Data")
                .cloned()
        );
    }

    #[test]
    fn dismember_instance_partition_overrides_weight_majority() {
        let (mut source, shape) = fixture();
        let skin_id = parse_skin_chain(&source, shape)
            .unwrap()
            .unwrap()
            .instance_block_id;
        let skin = &mut source.blocks[skin_id];
        skin.type_name = "BSDismemberSkinInstance".into();
        skin.set_field(
            "Partitions",
            NifValue::Array(vec![NifValue::Struct(indexmap::IndexMap::from([
                ("Body Part".to_string(), NifValue::UInt(3)),
                ("Part Flag".to_string(), NifValue::UInt(257)),
            ]))]),
        );
        assert_eq!(
            parse_skin_chain(&source, shape)
                .unwrap()
                .unwrap()
                .partitions[0]
                .body_part,
            3
        );
        let mut target = converted(&source);
        let plan = [
            CreatureBodyPartSegment {
                node: "Root".into(),
                index: 2,
                source_partition: Some(3),
            },
            CreatureBodyPartSegment {
                node: "Child".into(),
                index: 4,
                source_partition: None,
            },
        ];
        segment_source_creature_body(&source, &mut target, &plan).unwrap();
        assert_eq!(nonempty_segment(&target), 2);
    }

    #[test]
    fn legacy_caps_become_hidden_detached_and_stump_subsegments() {
        let (mut source, shape) = fixture();
        let instance = parse_skin_chain(&source, shape)
            .unwrap()
            .unwrap()
            .instance_block_id;
        let skin = &mut source.blocks[instance];
        skin.type_name = "BSDismemberSkinInstance".into();
        skin.set_field("Num Partitions", NifValue::UInt(3));
        skin.set_field(
            "Partitions",
            NifValue::Array(
                [3, 103, 203]
                    .map(|part| {
                        NifValue::Struct(indexmap::IndexMap::from([
                            ("Body Part".into(), NifValue::UInt(part)),
                            ("Part Flag".into(), NifValue::UInt(257)),
                        ]))
                    })
                    .to_vec(),
            ),
        );
        let data = source
            .blocks
            .iter_mut()
            .find(|block| block.type_name == "NiTriShapeData")
            .unwrap();
        data.set_field(
            "Triangles",
            NifValue::Array(vec![triangle_value([0, 1, 2]); 3]),
        );
        let partition = source
            .blocks
            .iter_mut()
            .find(|block| block.type_name == "NiSkinPartition")
            .unwrap();
        let Some(NifValue::Array(parts)) = partition.get_field("Partitions") else {
            panic!("partitions")
        };
        let parts = vec![parts[0].clone(); 3];
        partition.set_field("Num Partitions", NifValue::UInt(3));
        partition.set_field("Partitions", NifValue::Array(parts));
        let mut target = converted(&source);
        segment_source_creature_body(
            &source,
            &mut target,
            &[CreatureBodyPartSegment {
                node: "Root".into(),
                index: 2,
                source_partition: Some(3),
            }],
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("caps.nif");
        target.save(Some(path.clone())).unwrap();
        let target = NifFile::load(path).unwrap();
        let shape = target
            .blocks
            .iter()
            .find(|block| block.type_name == "BSSubIndexTriShape")
            .unwrap();
        let Some(NifValue::Array(parents)) = shape.get_field("Segment") else {
            panic!("segments")
        };
        let NifValue::Struct(parent) = &parents[2] else {
            panic!("parent")
        };
        assert_eq!(parent["Num Sub Segments"].as_i64(), 3);
        let Some(NifValue::Struct(shared)) = shape.get_field("Segment Data") else {
            panic!("shared data")
        };
        let NifValue::Array(starts) = &shared["Segment Starts"] else {
            panic!("starts")
        };
        let NifValue::Array(data) = &shared["Per Segment Data"] else {
            panic!("per segment data")
        };
        let offset = starts[2].as_i64() as usize;
        let user_indices = data[offset + 1..offset + 4]
            .iter()
            .map(|row| {
                let NifValue::Struct(fields) = row else {
                    panic!("row")
                };
                fields["User Index"].as_i64()
            })
            .collect::<Vec<_>>();
        assert_eq!(user_indices, [32, 100, 101]);
        assert_eq!(shape.get_field("Total Segments").unwrap().as_i64(), 36);
    }

    #[test]
    #[ignore = "requires the CLI-derived B21_CREATURE_REPLACEMENT_CORPUS manifest and extracted game assets"]
    fn real_body_part_replacements_preserve_skin_and_embedded_collision() {
        let manifest =
            std::path::PathBuf::from(std::env::var("B21_CREATURE_REPLACEMENT_CORPUS").unwrap());
        let cases: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        assert!(!cases.is_empty());
        let directory = tempfile::tempdir().unwrap();
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (index, case) in cases.iter().enumerate() {
            match super::super::skin_space_tests::audit_saved_skin(
                case,
                &directory.path().join(index.to_string()),
            ) {
                Ok(result) => results.push(result),
                Err(error) => failures
                    .push(serde_json::json!({"source_path":case["source_path"],"error":error})),
            }
        }
        let report = serde_json::json!({"models":cases.len(),"passed":results.len(),"failures":failures,"results":results});
        std::fs::write(
            manifest.with_file_name("creature_replacement_conversion_audit.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(
            failures.is_empty(),
            "{}",
            serde_json::to_string_pretty(&failures).unwrap()
        );
    }

    #[test]
    #[ignore = "requires B21_CREATURE_PART_CORPUS with extracted body NIFs and source BPTD segment plans"]
    fn real_creature_parts_preserve_geometry_and_bindings_after_save_reload() {
        let manifest = std::path::PathBuf::from(std::env::var("B21_CREATURE_PART_CORPUS").unwrap());
        let cases: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        assert!(!cases.is_empty());
        let temp = tempfile::tempdir().unwrap();
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (index, case) in cases.iter().enumerate() {
            match audit_saved_parts(case, &temp.path().join(index.to_string())) {
                Ok(result) => results.push(result),
                Err(error) => failures
                    .push(serde_json::json!({"source_path":case["source_path"],"error":error})),
            }
        }
        let report = serde_json::json!({"models":cases.len(),"passed":results.len(),"failures":failures,"results":results});
        let output = manifest.with_file_name("creature_mesh_body_part_audit.json");
        std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        eprintln!(
            "creature body-part mesh corpus: {} passed / {}; {}",
            results.len(),
            cases.len(),
            output.display()
        );
        assert!(
            failures.is_empty(),
            "{}",
            serde_json::to_string_pretty(&failures).unwrap()
        );
    }

    fn audit_saved_parts(
        case: &serde_json::Value,
        stage: &std::path::Path,
    ) -> Result<serde_json::Value, String> {
        let source_path = std::path::Path::new(case["source_path"].as_str().ok_or("source_path")?);
        let game = case["source_game"].as_str().ok_or("source_game")?;
        let plan: Vec<CreatureBodyPartSegment> =
            serde_json::from_value(case["segments"].clone()).map_err(|error| error.to_string())?;
        let source = NifFile::load(source_path).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(stage).map_err(|error| error.to_string())?;
        let path = stage.join("body.nif");
        crate::convert_file::stage_preserve_source_rig_nif(
            crate::convert_file::SourceRigNifKind::Body,
            source_path,
            &path,
            "Meshes/CreatureBodyPartAudit/body.nif",
            game,
            Some(&stage.join("materials")),
            &crate::convert_file::ConvertFileOptions::default(),
        )
        .map_err(|error| error.to_string())?;
        let before = NifFile::load(&path).map_err(|error| error.to_string())?;
        let mut segmented = before.clone();
        let assigned_shapes = segment_source_creature_body(&source, &mut segmented, &plan)?;
        segmented
            .save(Some(path.clone()))
            .map_err(|error| error.to_string())?;
        let saved = NifFile::load(&path).map_err(|error| error.to_string())?;
        if before.blocks.len() != saved.blocks.len() {
            return Err("segmentation changed block count".into());
        }
        let mut segment_triangles = BTreeMap::<usize, usize>::new();
        for (old, new) in before.blocks.iter().zip(&saved.blocks) {
            if old.type_name != new.type_name {
                return Err("segmentation changed block type".into());
            }
            if old.type_name.starts_with("BSSkin::") && old.fields != new.fields {
                return Err("segmentation changed skin binding".into());
            }
            if old.type_name != "BSSubIndexTriShape" {
                continue;
            }
            for field in ["Vertex Data", "Skin", "Translation", "Rotation", "Scale"] {
                if old.get_field(field) != new.get_field(field) {
                    return Err(format!("segmentation changed {field}"));
                }
            }
            let sorted = |shape: &crate::model::NifBlock| {
                let mut triangles = match shape.get_field("Triangles") {
                    Some(NifValue::Array(values)) => values
                        .iter()
                        .filter_map(super::super::triangle_from_value)
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                triangles.sort_unstable();
                triangles
            };
            let triangles = sorted(old);
            if triangles != sorted(new) {
                return Err("segmentation changed triangles".into());
            }
            let Some(NifValue::Array(segments)) = new.get_field("Segment") else {
                return Err("saved shape lacks segment table".into());
            };
            let mut start = 0;
            for (index, segment) in segments.iter().enumerate() {
                let NifValue::Struct(fields) = segment else {
                    return Err("saved segment is not a struct".into());
                };
                let offset = fields.get("Start Index").ok_or("segment start")?.as_i64() as usize;
                let count = fields
                    .get("Num Primitives")
                    .ok_or("segment count")?
                    .as_i64() as usize;
                if offset != start * 3 {
                    return Err("segment ranges overlap or have gaps".into());
                }
                start += count;
                *segment_triangles.entry(index).or_default() += count;
                if let Some(NifValue::Array(children)) = fields.get("Sub Segment") {
                    if !children.is_empty() {
                        let mut child_start = offset;
                        for child in children {
                            let NifValue::Struct(child) = child else {
                                return Err("invalid child segment".into());
                            };
                            if child.get("Start Index").ok_or("child start")?.as_i64() as usize
                                != child_start
                            {
                                return Err("child segments overlap or have gaps".into());
                            }
                            child_start +=
                                child.get("Num Primitives").ok_or("child count")?.as_i64() as usize
                                    * 3;
                        }
                        if child_start != offset + count * 3 {
                            return Err("child segments do not cover parent".into());
                        }
                    }
                }
            }
            if start != triangles.len() {
                return Err("segment ranges do not cover all triangles".into());
            }
        }
        Ok(
            serde_json::json!({"source_path":case["source_path"],"family_id":case["family_id"],"assigned_shapes":assigned_shapes,"segment_triangles":segment_triangles}),
        )
    }
}
