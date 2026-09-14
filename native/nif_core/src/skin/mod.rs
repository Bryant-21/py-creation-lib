//! Legacy-skin -> FO4 BSSubIndexTriShape conversion.
//!

pub mod bone_remap;
pub mod first_person;
pub mod pack;
pub mod segment;
pub mod source;
pub mod weight_transfer;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use indexmap::IndexMap;

use crate::model::{NifBlock, NifFile, NifValue};
use crate::skin::bone_remap::{
    SkeletonMap, VertexInfluences, body_part_to_fo4_segment, redistribute_unmapped,
};
use crate::skin::pack::{
    pack_skinned_vertex_data, pack_static_vertex_data, recompute_tangents_lengyel,
    vertex_desc_skinned, vertex_desc_static,
};
use crate::skin::segment::{SegmentSpec, build_segment_data};
use crate::skin::source::{LegacySkin, SkinParseError, SkinTransform, fold_partitions_to_global};
use crate::skin::weight_transfer::{MorphTransferConfig, transfer_morph_weights};

type ReferenceSkin = (Vec<[f32; 3]>, Vec<VertexInfluences>, Vec<String>);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LegacySkinPolicy {
    #[default]
    TranslateSkeleton,
    PreserveSourceRig,
}

pub fn rig_facegen_shape_from_reference(
    nif: &mut NifFile,
    shape_id: usize,
    reference: &NifFile,
) -> Result<(), String> {
    let shape = nif
        .get_block(shape_id)
        .cloned()
        .ok_or_else(|| format!("missing FaceGen shape {shape_id}"))?;
    let geometry = ShapeGeometry::from_shape(nif, &shape);
    if geometry.positions.is_empty() {
        return Err(format!("FaceGen shape {shape_id} has no vertices"));
    }

    let (reference_positions, reference_influences, reference_bones) =
        extract_reference_skin(reference);
    if reference_positions.is_empty()
        || reference_influences.len() != reference_positions.len()
        || reference_bones.is_empty()
    {
        return Err("FaceGen skin reference has no usable skinned vertices".to_string());
    }

    let mut target_bone_names = Vec::new();
    let mut target_bone_refs = Vec::new();
    let mut reference_to_target = vec![None; reference_bones.len()];
    for (reference_index, bone_name) in reference_bones.iter().enumerate() {
        let Some(target_ref) = find_node_by_name_case_insensitive(nif, bone_name) else {
            continue;
        };
        reference_to_target[reference_index] = Some(target_bone_names.len());
        target_bone_names.push(bone_name.clone());
        target_bone_refs.push(target_ref);
    }
    if target_bone_names.is_empty() {
        return Err(
            "FaceGen skin reference has no bones present in the target template".to_string(),
        );
    }

    let fallback_bone = target_bone_names
        .iter()
        .position(|name| name.eq_ignore_ascii_case("HEAD"))
        .unwrap_or(0);
    let mut influences = Vec::with_capacity(geometry.positions.len());
    for position in &geometry.positions {
        let nearest = reference_positions
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                distance_squared(**left, *position).total_cmp(&distance_squared(**right, *position))
            })
            .map(|(index, _)| index)
            .ok_or_else(|| "FaceGen skin reference has no vertices".to_string())?;
        let mut slots = reference_influences[nearest]
            .slots
            .iter()
            .filter_map(|(reference_bone, weight)| {
                let target_bone = reference_to_target
                    .get(*reference_bone)
                    .copied()
                    .flatten()?;
                (*weight > 0.0).then_some((target_bone, *weight))
            })
            .collect::<Vec<_>>();
        normalize_top_four(&mut slots);
        if slots.is_empty() {
            slots.push((fallback_bone, 1.0));
        }
        influences.push(VertexInfluences { slots });
    }

    let (tangents, bitangents) = if geometry.tangents_missing() {
        recompute_tangents_lengyel(
            &geometry.positions,
            &geometry.normals,
            &geometry.uvs,
            &geometry.triangles,
        )
    } else {
        (geometry.tangents.clone(), geometry.bitangents.clone())
    };
    let vertex_data = pack_skinned_vertex_data(
        &geometry.positions,
        &geometry.normals,
        &tangents,
        &bitangents,
        &geometry.uvs,
        geometry.vertex_colors.as_deref(),
        &influences,
    );
    let transforms = reference_bone_transforms(reference, &target_bone_names);
    let bone_data_id = create_bone_data(
        nif,
        target_bone_names.len(),
        &transforms,
        None,
        &geometry.positions,
        &influences,
    );
    let skeleton_root = nif
        .blocks
        .iter()
        .find(|block| {
            is_node_type(&block.type_name)
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name.trim_end_matches('\0').eq_ignore_ascii_case("BSFaceGenNiNodeSkinned"))
        })
        .map(|block| block.block_id)
        .ok_or_else(|| "target template has no BSFaceGenNiNodeSkinned root".to_string())?;

    let mut skin_fields = IndexMap::new();
    skin_fields.insert("Data".into(), NifValue::Ref(bone_data_id as i32));
    skin_fields.insert("Skin Partition".into(), NifValue::Ref(-1));
    skin_fields.insert("Skeleton Root".into(), NifValue::Ref(skeleton_root as i32));
    skin_fields.insert(
        "Num Bones".into(),
        NifValue::UInt(target_bone_refs.len() as u64),
    );
    skin_fields.insert(
        "Bones".into(),
        NifValue::Array(
            target_bone_refs
                .iter()
                .map(|id| NifValue::Ref(*id as i32))
                .collect(),
        ),
    );
    skin_fields.insert("Num Scales".into(), NifValue::UInt(0));
    skin_fields.insert("Scales".into(), NifValue::Array(Vec::new()));
    let skin_instance_id = nif.add_block("BSSkin::Instance", Some(skin_fields));

    let target_shape = nif
        .blocks
        .get_mut(shape_id)
        .ok_or_else(|| format!("missing copied FaceGen shape {shape_id}"))?;
    let flags = value_u64(target_shape.get_field("Flags")).unwrap_or(14) & !0x80000;
    target_shape.set_field("Flags", NifValue::UInt(flags));
    target_shape.set_field("Skin", NifValue::Ref(skin_instance_id as i32));
    target_shape.set_field("Skin Instance", NifValue::Ref(-1));
    target_shape.set_field(
        "Vertex Desc",
        NifValue::Int(vertex_desc_skinned(geometry.vertex_colors.is_some())),
    );
    target_shape.set_field("Num Vertices", NifValue::UInt(vertex_data.len() as u64));
    target_shape.set_field("Vertex Data", NifValue::Array(vertex_data));
    target_shape.set_field("Data Size", NifValue::UInt(data_size(target_shape) as u64));
    mark_shader_skinned(nif, shape_id);
    nif.rebuild_header();
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct ConvertLegacySkinResult {
    pub shapes_skinned: usize,
    pub vertices_repacked: usize,
    pub bones_remapped: usize,
    pub bones_dropped_unmapped: usize,
    pub weights_redistributed: usize,
    pub vertices_morph_weighted: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertLegacySkinError {
    #[error("skeleton map: {0}")]
    SkeletonMap(#[from] bone_remap::SkeletonMapError),
    #[error("skin parse: {0}")]
    SkinParse(#[from] SkinParseError),
    #[error("skeleton translation requires translation_maps_dir")]
    MissingTranslationMaps,
    #[error("preserve-source-rig skin: {0}")]
    PreserveSourceRig(String),
}

pub fn convert_legacy_skin(
    nif: &mut NifFile,
    translation_maps_dir: &Path,
    reference_body: Option<&Path>,
    morph_weight_cap: f32,
) -> Result<ConvertLegacySkinResult, ConvertLegacySkinError> {
    convert_legacy_skin_for_games(
        nif,
        translation_maps_dir,
        "fnv",
        "fo4",
        reference_body,
        morph_weight_cap,
    )
}

pub fn convert_legacy_skin_for_games(
    nif: &mut NifFile,
    translation_maps_dir: &Path,
    source_game: &str,
    target_game: &str,
    reference_body: Option<&Path>,
    morph_weight_cap: f32,
) -> Result<ConvertLegacySkinResult, ConvertLegacySkinError> {
    convert_legacy_skin_for_games_with_policy(
        nif,
        Some(translation_maps_dir),
        source_game,
        target_game,
        reference_body,
        morph_weight_cap,
        LegacySkinPolicy::TranslateSkeleton,
    )
}

pub fn convert_legacy_skin_for_games_with_policy(
    nif: &mut NifFile,
    translation_maps_dir: Option<&Path>,
    source_game: &str,
    target_game: &str,
    reference_body: Option<&Path>,
    morph_weight_cap: f32,
    policy: LegacySkinPolicy,
) -> Result<ConvertLegacySkinResult, ConvertLegacySkinError> {
    let mut working = nif.clone();
    let result = convert_legacy_skin_for_games_in_place(
        &mut working,
        translation_maps_dir,
        source_game,
        target_game,
        reference_body,
        morph_weight_cap,
        policy,
    )?;
    *nif = working;
    Ok(result)
}

pub fn convert_unskinned_legacy_shapes(nif: &mut NifFile) -> usize {
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| matches!(block.type_name.as_str(), "NiTriShape" | "NiTriStrips"))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut converted = 0;
    let mut remove_blocks = HashSet::new();

    for shape_id in shape_ids {
        let Some(shape) = nif.get_block(shape_id).cloned() else {
            continue;
        };
        if value_ref(shape.get_field("Skin Instance")).is_some_and(|skin| skin >= 0)
            || value_ref(shape.get_field("Skin")).is_some_and(|skin| skin >= 0)
        {
            continue;
        }
        let Some(data_id) = legacy_geometry_data_ref(nif, shape_id) else {
            continue;
        };
        let geometry = ShapeGeometry::from_shape(nif, &shape);
        let (shader_ref, alpha_ref) = geometry_property_refs(nif, &shape);
        if convert_static_shape(nif, shape_id, geometry, shader_ref, alpha_ref).is_some() {
            converted += 1;
            remove_blocks.insert(data_id);
        }
    }

    if !remove_blocks.is_empty() {
        let mut ids = remove_blocks.into_iter().collect::<Vec<_>>();
        ids.sort_unstable();
        nif.remove_blocks(&ids);
        nif.rebuild_header();
    }

    converted
}

fn convert_legacy_skin_for_games_in_place(
    nif: &mut NifFile,
    translation_maps_dir: Option<&Path>,
    source_game: &str,
    target_game: &str,
    reference_body: Option<&Path>,
    morph_weight_cap: f32,
    policy: LegacySkinPolicy,
) -> Result<ConvertLegacySkinResult, ConvertLegacySkinError> {
    let map = match policy {
        LegacySkinPolicy::TranslateSkeleton => Some(SkeletonMap::load(
            translation_maps_dir.ok_or(ConvertLegacySkinError::MissingTranslationMaps)?,
            source_game,
            target_game,
        )?),
        LegacySkinPolicy::PreserveSourceRig => None,
    };
    let reference_skin = matches!(policy, LegacySkinPolicy::TranslateSkeleton)
        .then_some(reference_body)
        .flatten()
        .and_then(|path| NifFile::load(path.to_path_buf()).ok())
        .map(|reference| extract_reference_skin(&reference));
    let shape_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSTriShape"
                    | "BSSubIndexTriShape"
                    | "BSDynamicTriShape"
                    | "NiTriShape"
                    | "NiTriStrips"
            )
        })
        .map(|block| block.block_id)
        .collect();

    let mut parsed_shapes = Vec::new();
    for shape_id in shape_ids {
        let Some(parsed) = source::parse_skin_chain(nif, shape_id)? else {
            continue;
        };
        if matches!(policy, LegacySkinPolicy::PreserveSourceRig) {
            validate_preserved_source_rig_input(nif, shape_id, &parsed)?;
        }
        parsed_shapes.push((shape_id, parsed));
    }

    let mut result = ConvertLegacySkinResult::default();
    let mut remove_blocks = HashSet::new();

    for (shape_id, parsed) in parsed_shapes {
        let converted = convert_one_shape(
            nif,
            shape_id,
            &parsed,
            map.as_ref(),
            source_game,
            reference_skin.as_ref(),
            morph_weight_cap,
            policy,
        );
        match converted {
            Some(converted) => {
                if converted.skinned {
                    result.shapes_skinned += 1;
                }
                result.vertices_repacked += converted.vertices_repacked;
                result.bones_remapped += converted.bones_remapped;
                result.bones_dropped_unmapped += converted.bones_dropped_unmapped;
                result.weights_redistributed += converted.weights_redistributed;
                result.vertices_morph_weighted += converted.vertices_morph_weighted;
                if let Some(data_id) = legacy_geometry_data_ref(nif, shape_id) {
                    remove_blocks.insert(data_id);
                }
                remove_blocks.insert(parsed.instance_block_id);
                remove_blocks.insert(parsed.skin_data_block_id);
                if let Some(partition_id) = parsed.skin_partition_block_id {
                    remove_blocks.insert(partition_id);
                }
            }
            None => result.warnings.push(format!(
                "shape {shape_id}: no geometry; skipped skin conversion"
            )),
        }
    }

    if !remove_blocks.is_empty() {
        let mut ids: Vec<usize> = remove_blocks.into_iter().collect();
        ids.sort_unstable();
        nif.remove_blocks(&ids);
    }
    if result.shapes_skinned > 0 {
        if matches!(policy, LegacySkinPolicy::TranslateSkeleton) {
            restructure_bone_tree(nif);
        }
        if matches!(policy, LegacySkinPolicy::PreserveSourceRig) {
            validate_preserved_source_rig_output(nif)?;
        }
        nif.rebuild_header();
    }

    Ok(result)
}

#[derive(Debug, Clone, Default)]
struct ShapeConversion {
    skinned: bool,
    vertices_repacked: usize,
    bones_remapped: usize,
    bones_dropped_unmapped: usize,
    weights_redistributed: usize,
    vertices_morph_weighted: usize,
}

fn convert_one_shape(
    nif: &mut NifFile,
    shape_id: usize,
    parsed: &LegacySkin,
    map: Option<&SkeletonMap>,
    source_game: &str,
    reference_skin: Option<&ReferenceSkin>,
    morph_weight_cap: f32,
    policy: LegacySkinPolicy,
) -> Option<ShapeConversion> {
    let shape = nif.get_block(shape_id)?.clone();
    let mut geometry = ShapeGeometry::from_shape(nif, &shape);
    geometry.complete_partition_geometry(nif, &shape, parsed);
    geometry.apply_transform(&parsed.skin_transform);
    let (shader_ref, alpha_ref) = geometry_property_refs(nif, &shape);
    let source_bone_refs = source_bone_refs(nif, parsed.instance_block_id);
    let (source_to_local, mut target_bone_names, mut target_bone_refs, bones_remapped) =
        match policy {
            LegacySkinPolicy::TranslateSkeleton => {
                build_target_bones(nif, parsed, &source_bone_refs, map?)
            }
            LegacySkinPolicy::PreserveSourceRig => {
                preserve_source_bones(nif, parsed, &source_bone_refs)?
            }
        };
    if target_bone_refs.is_empty() {
        return convert_static_shape(nif, shape_id, geometry, shader_ref, alpha_ref);
    }

    let (mut influences, bones_dropped_unmapped, weights_redistributed) = match policy {
        LegacySkinPolicy::TranslateSkeleton => {
            let mut influences = seed_influences(parsed, geometry.positions.len());
            let redistribute_report = redistribute_unmapped(&mut influences, &parsed.bones, map?);
            remap_influences_to_local(&mut influences, &source_to_local);
            (
                influences,
                redistribute_report.dropped_unmapped.len(),
                redistribute_report.weights_redistributed,
            )
        }
        LegacySkinPolicy::PreserveSourceRig => (
            source_rig_influences(parsed, geometry.positions.len())?,
            0,
            0,
        ),
    };

    let mut conversion = ShapeConversion {
        skinned: true,
        vertices_repacked: geometry.positions.len(),
        bones_remapped,
        bones_dropped_unmapped,
        weights_redistributed,
        vertices_morph_weighted: 0,
    };

    if let Some((ref_positions, ref_influences, ref_bones)) = reference_skin {
        let cfg = MorphTransferConfig {
            morph_weight_cap,
            k_neighbors: 4,
        };
        let stats = transfer_morph_weights(
            &geometry.positions,
            &mut influences,
            &mut target_bone_names,
            ref_positions,
            ref_influences,
            ref_bones,
            &cfg,
        );
        conversion.vertices_morph_weighted += stats.vertices_morph_weighted;
        while target_bone_refs.len() < target_bone_names.len() {
            let name = target_bone_names[target_bone_refs.len()].clone();
            let bone_ref = ensure_named_bone(nif, &name, None);
            target_bone_refs.push(bone_ref);
        }
    }

    let (tangents, bitangents) = if geometry.tangents_missing() {
        recompute_tangents_lengyel(
            &geometry.positions,
            &geometry.normals,
            &geometry.uvs,
            &geometry.triangles,
        )
    } else {
        (geometry.tangents.clone(), geometry.bitangents.clone())
    };

    let vertex_data = pack_skinned_vertex_data(
        &geometry.positions,
        &geometry.normals,
        &tangents,
        &bitangents,
        &geometry.uvs,
        geometry.vertex_colors.as_deref(),
        &influences,
    );
    let skin_instance_id = create_skin_instance(
        nif,
        parsed,
        &target_bone_refs,
        &target_bone_names,
        &local_bone_transforms(
            &parsed.bone_transforms,
            &source_to_local,
            target_bone_names.len(),
        ),
        matches!(policy, LegacySkinPolicy::PreserveSourceRig)
            .then_some(parsed.bone_bounds.as_slice()),
        &geometry.positions,
        &influences,
    );

    if let Some(target_shape) = nif.blocks.get_mut(shape_id) {
        target_shape.type_name = "BSSubIndexTriShape".to_string();
        target_shape.fields.shift_remove("Dynamic Data Size");
        target_shape.fields.shift_remove("Vertices");
        target_shape.set_field("Skin", NifValue::Ref(skin_instance_id as i32));
        target_shape.set_field("Skin Instance", NifValue::Ref(-1));
        if let Some(bound) = geometry.bounding_sphere.clone() {
            target_shape.set_field("Bounding Sphere", bound);
        }
        target_shape.set_field("Shader Property", NifValue::Ref(shader_ref));
        target_shape.set_field("Alpha Property", NifValue::Ref(alpha_ref));
        target_shape.set_field(
            "Vertex Desc",
            NifValue::Int(vertex_desc_skinned(geometry.vertex_colors.is_some())),
        );
        target_shape.set_field("Num Vertices", NifValue::UInt(vertex_data.len() as u64));
        target_shape.set_field(
            "Num Triangles",
            NifValue::UInt(geometry.triangles.len() as u64),
        );
        target_shape.set_field("Vertex Data", NifValue::Array(vertex_data));
        target_shape.set_field(
            "Triangles",
            NifValue::Array(
                geometry
                    .triangles
                    .iter()
                    .copied()
                    .map(triangle_value)
                    .collect(),
            ),
        );
        target_shape.set_field("Data Size", NifValue::UInt(data_size(target_shape) as u64));
        write_segments(
            target_shape,
            parsed,
            &geometry.triangles,
            source_game,
            policy,
        );
    }
    mark_shader_skinned(nif, shape_id);

    Some(conversion)
}

fn convert_static_shape(
    nif: &mut NifFile,
    shape_id: usize,
    geometry: ShapeGeometry,
    shader_ref: i32,
    alpha_ref: i32,
) -> Option<ShapeConversion> {
    if geometry.positions.is_empty() {
        return None;
    }
    let (tangents, bitangents) = if geometry.tangents_missing() {
        recompute_tangents_lengyel(
            &geometry.positions,
            &geometry.normals,
            &geometry.uvs,
            &geometry.triangles,
        )
    } else {
        (geometry.tangents.clone(), geometry.bitangents.clone())
    };
    let vertex_data = pack_static_vertex_data(
        &geometry.positions,
        &geometry.normals,
        &tangents,
        &bitangents,
        &geometry.uvs,
        geometry.vertex_colors.as_deref(),
    );

    if let Some(target_shape) = nif.blocks.get_mut(shape_id) {
        target_shape.type_name = "BSSubIndexTriShape".to_string();
        target_shape.set_field("Skin", NifValue::Ref(-1));
        target_shape.set_field("Skin Instance", NifValue::Ref(-1));
        if let Some(bound) = geometry.bounding_sphere.clone() {
            target_shape.set_field("Bounding Sphere", bound);
        }
        target_shape.set_field("Shader Property", NifValue::Ref(shader_ref));
        target_shape.set_field("Alpha Property", NifValue::Ref(alpha_ref));
        target_shape.set_field(
            "Vertex Desc",
            NifValue::Int(vertex_desc_static(geometry.vertex_colors.is_some())),
        );
        target_shape.set_field("Num Vertices", NifValue::UInt(vertex_data.len() as u64));
        target_shape.set_field(
            "Num Triangles",
            NifValue::UInt(geometry.triangles.len() as u64),
        );
        target_shape.set_field("Vertex Data", NifValue::Array(vertex_data));
        target_shape.set_field(
            "Triangles",
            NifValue::Array(
                geometry
                    .triangles
                    .iter()
                    .copied()
                    .map(triangle_value)
                    .collect(),
            ),
        );
        target_shape.set_field("Data Size", NifValue::UInt(data_size(target_shape) as u64));
        write_segments_with_user_index(target_shape, geometry.triangles.len(), 32);
    }

    Some(ShapeConversion {
        skinned: false,
        vertices_repacked: geometry.positions.len(),
        bones_remapped: 0,
        bones_dropped_unmapped: 0,
        weights_redistributed: 0,
        vertices_morph_weighted: 0,
    })
}

pub fn restructure_bone_tree(nif: &mut NifFile) {
    if nif.blocks.is_empty() {
        return;
    }
    let root_id = nif
        .header
        .footer_roots
        .iter()
        .find_map(|id| (*id >= 0).then_some(*id as usize))
        .unwrap_or(0);

    let required_bones: Vec<i32> = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSSkin::Instance" | "BSDismemberSkinInstance"
            )
        })
        .flat_map(|block| ref_array(block.get_field("Bones")))
        .filter(|id| *id >= 0)
        .collect();
    if required_bones.is_empty() {
        return;
    }

    let Some(root) = nif.blocks.get_mut(root_id) else {
        return;
    };
    let mut children = value_array(root.get_field("Children"));
    let mut existing: HashSet<i32> = children
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .collect();
    let mut changed = false;
    for bone_id in required_bones {
        if bone_id as usize == root_id || !existing.insert(bone_id) {
            continue;
        }
        children.push(NifValue::Ref(bone_id));
        changed = true;
    }
    if changed {
        root.set_field("Num Children", NifValue::UInt(children.len() as u64));
        root.set_field("Children", NifValue::Array(children));
    }
}

fn build_target_bones(
    nif: &mut NifFile,
    parsed: &LegacySkin,
    source_bone_refs: &[i32],
    map: &SkeletonMap,
) -> (Vec<Option<usize>>, Vec<String>, Vec<usize>, usize) {
    let mut source_to_local = vec![None; parsed.bones.len()];
    let mut target_bone_names = Vec::new();
    let mut target_bone_refs = Vec::new();
    let mut target_lookup = HashMap::new();
    let mut bones_remapped = 0usize;

    for (source_index, bone) in parsed.bones.iter().enumerate() {
        let Some(target_name) = map.lookup(&bone.name).map(str::to_string) else {
            continue;
        };
        bones_remapped += 1;
        let local_index = match target_lookup.get(&target_name).copied() {
            Some(index) => index,
            None => {
                let source_ref = source_bone_refs
                    .get(source_index)
                    .copied()
                    .filter(|id| *id >= 0)
                    .map(|id| id as usize);
                let bone_ref = ensure_named_bone(nif, &target_name, source_ref);
                let index = target_bone_names.len();
                target_bone_names.push(target_name.clone());
                target_bone_refs.push(bone_ref);
                target_lookup.insert(target_name, index);
                index
            }
        };
        source_to_local[source_index] = Some(local_index);
    }

    (
        source_to_local,
        target_bone_names,
        target_bone_refs,
        bones_remapped,
    )
}

fn preserve_source_bones(
    nif: &NifFile,
    parsed: &LegacySkin,
    source_bone_refs: &[i32],
) -> Option<(Vec<Option<usize>>, Vec<String>, Vec<usize>, usize)> {
    if source_bone_refs.len() != parsed.bones.len() {
        return None;
    }
    let mut names = Vec::with_capacity(source_bone_refs.len());
    let mut refs = Vec::with_capacity(source_bone_refs.len());
    for (source_ref, bone) in source_bone_refs.iter().zip(&parsed.bones) {
        let source_ref = usize::try_from(*source_ref).ok()?;
        let node = nif.get_block(source_ref)?;
        if !is_node_type(&node.type_name) || bone.name.is_empty() {
            return None;
        }
        names.push(bone.name.clone());
        refs.push(source_ref);
    }
    Some(((0..refs.len()).map(Some).collect(), names, refs, 0))
}

fn source_rig_influences(
    parsed: &LegacySkin,
    vertex_count: usize,
) -> Option<Vec<VertexInfluences>> {
    let mut influences = vec![VertexInfluences::default(); vertex_count];
    for partition in &parsed.partitions {
        if partition.vertex_map.len() != partition.influences.len() {
            return None;
        }
        for (local_vertex, row) in partition.influences.iter().enumerate() {
            let global_vertex = *partition.vertex_map.get(local_vertex)? as usize;
            let destination = influences.get_mut(global_vertex)?;
            if row.len() != 4 {
                return None;
            }
            let mut slots = Vec::with_capacity(row.len());
            for (local_bone, weight) in row {
                let global_bone = *partition.bones.get(*local_bone as usize)? as usize;
                if global_bone >= parsed.bones.len() || !weight.is_finite() || *weight < 0.0 {
                    return None;
                }
                slots.push((global_bone, *weight));
            }
            normalize_lanes(&mut slots)?;
            if !destination.slots.is_empty() {
                if !source_rig_lanes_equivalent(&destination.slots, &slots) {
                    return None;
                }
                continue;
            }
            destination.slots = slots;
        }
    }
    influences
        .iter()
        .all(|influence| !influence.slots.is_empty())
        .then_some(influences)
}

fn source_rig_lanes_equivalent(left: &[(usize, f32)], right: &[(usize, f32)]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|((left_bone, left_weight), (right_bone, right_weight))| {
                (left_weight - right_weight).abs() <= 1e-4
                    && (*left_weight <= 1e-4 && *right_weight <= 1e-4 || left_bone == right_bone)
            })
}

fn normalize_lanes(slots: &mut [(usize, f32)]) -> Option<()> {
    let total = slots.iter().map(|(_, weight)| *weight).sum::<f32>();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    for (_, weight) in slots {
        *weight /= total;
    }
    Some(())
}

fn validate_preserved_source_rig_input(
    nif: &NifFile,
    shape_id: usize,
    parsed: &LegacySkin,
) -> Result<(), ConvertLegacySkinError> {
    let shape = nif.get_block(shape_id).ok_or_else(|| {
        ConvertLegacySkinError::PreserveSourceRig(format!("shape {shape_id} is missing"))
    })?;
    let source_refs = source_bone_refs(nif, parsed.instance_block_id);
    let declared_bones = nif
        .get_block(parsed.instance_block_id)
        .and_then(|instance| value_u64(instance.get_field("Num Bones")))
        .unwrap_or(source_refs.len() as u64) as usize;
    if source_refs.len() != declared_bones || parsed.bones.len() != declared_bones {
        return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
            "shape {shape_id} palette declares {declared_bones} bones but resolves {} refs and {} names",
            source_refs.len(),
            parsed.bones.len()
        )));
    }
    if parsed.bone_transforms.len() != declared_bones || parsed.bone_bounds.len() != declared_bones
    {
        return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
            "shape {shape_id} has incomplete bind data"
        )));
    }
    for (palette_index, (source_ref, bone)) in source_refs.iter().zip(&parsed.bones).enumerate() {
        let source_ref = usize::try_from(*source_ref).map_err(|_| {
            ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {shape_id} palette bone {palette_index} has negative ref"
            ))
        })?;
        let node = nif.get_block(source_ref).ok_or_else(|| {
            ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {shape_id} palette bone {palette_index} ref {source_ref} is missing"
            ))
        })?;
        let node_name = match node.get_field("Name") {
            Some(NifValue::String(name)) => name.trim_end_matches('\0'),
            _ => "",
        };
        if !is_node_type(&node.type_name) || node_name != bone.name {
            return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {shape_id} palette bone {palette_index} does not resolve to {:?}",
                bone.name
            )));
        }
        if parsed.bone_bounds[palette_index]
            .as_ref()
            .and_then(sphere_value)
            .is_none()
        {
            return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {shape_id} palette bone {palette_index} has no finite bound"
            )));
        }
    }

    let mut geometry = ShapeGeometry::from_shape(nif, shape);
    geometry.complete_partition_geometry(nif, shape, parsed);
    if geometry.positions.is_empty() {
        return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
            "shape {shape_id} has no partition vertex stream"
        )));
    }
    let partition_triangles_are_global = partition_triangles_use_global_vertices(nif, parsed);
    for partition in &parsed.partitions {
        for triangle in &partition.triangles {
            let limit = if partition_triangles_are_global {
                geometry.positions.len()
            } else {
                partition.vertex_map.len()
            };
            if triangle.iter().any(|index| *index as usize >= limit) {
                return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                    "shape {shape_id} has an out-of-range partition triangle"
                )));
            }
        }
    }
    source_rig_influences(parsed, geometry.positions.len()).ok_or_else(|| {
        ConvertLegacySkinError::PreserveSourceRig(format!(
            "shape {shape_id} has invalid palette indices, vertex map, or weight lanes"
        ))
    })?;
    Ok(())
}

fn validate_preserved_source_rig_output(nif: &NifFile) -> Result<(), ConvertLegacySkinError> {
    if let Some(block) = nif.blocks.iter().find(|block| {
        matches!(
            block.type_name.as_str(),
            "NiSkinInstance" | "BSDismemberSkinInstance" | "NiSkinData" | "NiSkinPartition"
        )
    }) {
        return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
            "legacy skin block {} ({}) remains",
            block.block_id, block.type_name
        )));
    }

    for shape in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
    {
        let Some(skin_id) = value_ref(shape.get_field("Skin")).filter(|id| *id >= 0) else {
            continue;
        };
        let skin = nif.get_block(skin_id as usize).ok_or_else(|| {
            ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {} has missing BSSkin ref {skin_id}",
                shape.block_id
            ))
        })?;
        if skin.type_name != "BSSkin::Instance" {
            return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {} skin ref is {}",
                shape.block_id, skin.type_name
            )));
        }
        let bones = ref_array(skin.get_field("Bones"));
        if bones.is_empty()
            || bones.iter().any(|bone| {
                *bone < 0
                    || nif
                        .get_block(*bone as usize)
                        .is_none_or(|block| !is_node_type(&block.type_name))
            })
        {
            return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                "shape {} has an unresolved palette bone",
                shape.block_id
            )));
        }
        for (vertex_index, vertex) in value_array(shape.get_field("Vertex Data"))
            .iter()
            .enumerate()
        {
            let NifValue::Struct(fields) = vertex else {
                return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                    "shape {} vertex {vertex_index} is not structured",
                    shape.block_id
                )));
            };
            let indices = numeric_array(fields.get("Bone Indices"));
            let weights = float_array(fields.get("Bone Weights"));
            if indices.len() != 4
                || weights.len() != 4
                || indices.iter().zip(&weights).any(|(bone, weight)| {
                    !weight.is_finite() || *weight < 0.0 || *bone as usize >= bones.len()
                })
                || (weights.iter().sum::<f32>() - 1.0).abs() > 1e-4
            {
                return Err(ConvertLegacySkinError::PreserveSourceRig(format!(
                    "shape {} vertex {vertex_index} has invalid weight lanes",
                    shape.block_id
                )));
            }
        }
    }
    Ok(())
}

fn ensure_named_bone(
    nif: &mut NifFile,
    target_name: &str,
    reusable_source: Option<usize>,
) -> usize {
    if let Some(existing_id) = find_node_by_name(nif, target_name) {
        return existing_id;
    }
    if let Some(source_id) = reusable_source {
        if let Some(block) = nif.blocks.get_mut(source_id) {
            block.set_field("Name", NifValue::String(target_name.to_string()));
            return source_id;
        }
    }

    let mut fields = IndexMap::new();
    fields.insert("Name".into(), NifValue::String(target_name.to_string()));
    fields.insert("Flags".into(), NifValue::UInt(14));
    fields.insert("Translation".into(), NifValue::Vec3([0.0, 0.0, 0.0]));
    fields.insert(
        "Rotation".into(),
        NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
    );
    fields.insert("Scale".into(), NifValue::Float(1.0));
    fields.insert("Num Children".into(), NifValue::UInt(0));
    fields.insert("Children".into(), NifValue::Array(Vec::new()));
    nif.add_block("NiNode", Some(fields))
}

fn create_skin_instance(
    nif: &mut NifFile,
    parsed: &LegacySkin,
    bone_refs: &[usize],
    bone_names: &[String],
    bone_transforms: &[SkinTransform],
    source_bone_bounds: Option<&[Option<NifValue>]>,
    positions: &[[f32; 3]],
    influences: &[VertexInfluences],
) -> usize {
    let bone_data_id = create_bone_data(
        nif,
        bone_names.len(),
        bone_transforms,
        source_bone_bounds,
        positions,
        influences,
    );
    let bone_ref_values = bone_refs
        .iter()
        .map(|id| NifValue::Ref(*id as i32))
        .collect::<Vec<_>>();

    let mut fields = IndexMap::new();
    fields.insert("Data".into(), NifValue::Ref(bone_data_id as i32));
    fields.insert("Skin Partition".into(), NifValue::Ref(-1));
    fields.insert(
        "Skeleton Root".into(),
        NifValue::Ref(skin_skeleton_root_ref(nif, parsed)),
    );
    fields.insert(
        "Num Bones".into(),
        NifValue::UInt(bone_ref_values.len() as u64),
    );
    fields.insert("Bones".into(), NifValue::Array(bone_ref_values));

    // Both kinds emit BSSkin::Instance: FO4 has no BSDismemberSkinInstance RTTI
    // entry, and dismemberment already rides on the shape's segments (see
    // `write_segments` / `segment_specs_from_partitions`, fed by the same
    // `parsed.partitions`).
    fields.insert("Num Scales".into(), NifValue::UInt(0));
    fields.insert("Scales".into(), NifValue::Array(Vec::new()));
    nif.add_block("BSSkin::Instance", Some(fields))
}

fn skin_skeleton_root_ref(nif: &NifFile, parsed: &LegacySkin) -> i32 {
    if let Some(root) = parsed
        .skeleton_root
        .filter(|root| nif.get_block(*root).is_some())
    {
        return root as i32;
    }
    nif.header
        .footer_roots
        .iter()
        .copied()
        .find(|root| *root >= 0 && nif.get_block(*root as usize).is_some())
        .unwrap_or(0)
}

fn create_bone_data(
    nif: &mut NifFile,
    bone_count: usize,
    bone_transforms: &[SkinTransform],
    source_bone_bounds: Option<&[Option<NifValue>]>,
    positions: &[[f32; 3]],
    influences: &[VertexInfluences],
) -> usize {
    let bone_list = (0..bone_count)
        .map(|bone_index| {
            bone_data_entry(
                source_bone_bounds
                    .and_then(|bounds| bounds.get(bone_index))
                    .and_then(Option::as_ref)
                    .and_then(sphere_value)
                    .unwrap_or_else(|| bone_bounds(bone_index, positions, influences)),
                bone_transforms.get(bone_index).copied().unwrap_or_default(),
            )
        })
        .collect();
    let mut fields = IndexMap::new();
    fields.insert("Num Bones".into(), NifValue::UInt(bone_count as u64));
    fields.insert("Bone List".into(), NifValue::Array(bone_list));
    nif.add_block("BSSkin::BoneData", Some(fields))
}

fn seed_influences(parsed: &LegacySkin, vertex_count: usize) -> Vec<VertexInfluences> {
    let mut influences = fold_partitions_to_global(&parsed.partitions, vertex_count);
    if influences.len() < vertex_count {
        influences.resize_with(vertex_count, VertexInfluences::default);
    }

    for (vertex_index, data_influence) in parsed.data_influences.iter().enumerate() {
        let Some(influence) = influences.get_mut(vertex_index) else {
            break;
        };
        if data_influence.slots.is_empty() {
            continue;
        }
        if influence.slots.is_empty() {
            *influence = data_influence.clone();
            continue;
        }

        let mut supplemented = false;
        for (bone_index, weight) in &data_influence.slots {
            if influence
                .slots
                .iter()
                .any(|(existing_bone, _)| existing_bone == bone_index)
            {
                continue;
            }
            influence.slots.push((*bone_index, *weight));
            supplemented = true;
        }
        if supplemented {
            normalize_top_four(&mut influence.slots);
        }
    }

    influences
}

fn local_bone_transforms(
    source_transforms: &[SkinTransform],
    source_to_local: &[Option<usize>],
    local_count: usize,
) -> Vec<SkinTransform> {
    let mut local_transforms = vec![SkinTransform::identity(); local_count];
    for (source_index, local_index) in source_to_local.iter().enumerate() {
        let Some(local_index) = local_index else {
            continue;
        };
        let Some(transform) = source_transforms.get(source_index).copied() else {
            continue;
        };
        if let Some(slot) = local_transforms.get_mut(*local_index) {
            *slot = transform;
        }
    }
    local_transforms
}

fn remap_influences_to_local(
    influences: &mut [VertexInfluences],
    source_to_local: &[Option<usize>],
) {
    for influence in influences {
        let mut slots = Vec::new();
        for (source_bone_index, weight) in influence.slots.drain(..) {
            let Some(Some(local_index)) = source_to_local.get(source_bone_index) else {
                continue;
            };
            merge_slot(&mut slots, *local_index, weight);
        }
        if slots.is_empty() && source_to_local.iter().any(Option::is_some) {
            slots.push((0, 1.0));
        }
        normalize_top_four(&mut slots);
        influence.slots = slots;
    }
}

fn write_segments(
    shape: &mut NifBlock,
    parsed: &LegacySkin,
    triangles: &[[u32; 3]],
    source_game: &str,
    policy: LegacySkinPolicy,
) {
    if matches!(source_game, "skyrimse" | "fnv" | "fo3")
        && matches!(policy, LegacySkinPolicy::PreserveSourceRig)
    {
        write_root_only_creature_segments(shape, triangles.len());
        return;
    }
    let specs = segment_specs_from_partitions(parsed, triangles, source_game);
    write_segment_specs(shape, triangles.len(), &specs);
}

const ROOT_ONLY_CREATURE_SEGMENT_INDEX: usize = 32;

fn write_root_only_creature_segments(shape: &mut NifBlock, triangle_count: usize) {
    // BPTD indexes the serialized top-level segment array, so segment 32 requires 33 entries.
    let specs = (0..=ROOT_ONLY_CREATURE_SEGMENT_INDEX)
        .map(|segment_index| SegmentSpec {
            triangle_start: 0,
            triangle_count: if segment_index == ROOT_ONLY_CREATURE_SEGMENT_INDEX {
                triangle_count as u32
            } else {
                0
            },
            user_index: segment_index as u32,
        })
        .collect::<Vec<_>>();
    write_segment_specs(shape, triangle_count, &specs);
}

fn write_segments_with_user_index(shape: &mut NifBlock, triangle_count: usize, user_index: u32) {
    let specs = [SegmentSpec {
        triangle_start: 0,
        triangle_count: triangle_count as u32,
        user_index,
    }];
    write_segment_specs(shape, triangle_count, &specs);
}

fn write_segment_specs(shape: &mut NifBlock, triangle_count: usize, specs: &[SegmentSpec]) {
    let fallback_specs;
    let specs = if specs.is_empty() {
        fallback_specs = [SegmentSpec {
            triangle_start: 0,
            triangle_count: triangle_count as u32,
            user_index: 32,
        }];
        &fallback_specs[..]
    } else {
        specs
    };
    let (num_segments, segments, total_segments) = build_segment_data(specs);
    shape.set_field("Num Primitives", NifValue::UInt(triangle_count as u64));
    shape.set_field("Num Segments", NifValue::UInt(num_segments as u64));
    // FO4 requires Total Segments == Num Segments + sum(Num Sub Segments); we
    // emit no sub segments, so the shared Segment Data block must be absent too
    // (its presence is keyed off Num Segments < Total Segments). Inflating the
    // total desyncs the engine mid-block and corrupts the next block's string
    // index — an access violation in NiStringExtraData::LoadBinary.
    shape.set_field("Total Segments", NifValue::UInt(total_segments as u64));
    shape.set_field("Segment", segments);
    shape.fields.shift_remove("Segment Data");
}

fn segment_specs_from_partitions(
    parsed: &LegacySkin,
    triangles: &[[u32; 3]],
    source_game: &str,
) -> Vec<SegmentSpec> {
    if triangles.is_empty() {
        return Vec::new();
    }

    let mut triangle_lookup = HashMap::<[u32; 3], Vec<usize>>::new();
    for (triangle_index, triangle) in triangles.iter().copied().enumerate() {
        triangle_lookup
            .entry(sorted_triangle(triangle))
            .or_default()
            .push(triangle_index);
    }

    let mut user_indices = vec![None::<u32>; triangles.len()];
    for partition in &parsed.partitions {
        let Some(remap) = body_part_to_fo4_segment(source_game, partition.body_part) else {
            continue;
        };
        if partition.triangles.is_empty() {
            let partition_vertices = partition.vertex_map.iter().copied().collect::<HashSet<_>>();
            for (triangle_index, triangle) in triangles.iter().enumerate() {
                if user_indices[triangle_index].is_none()
                    && triangle
                        .iter()
                        .all(|vertex_index| partition_vertices.contains(vertex_index))
                {
                    user_indices[triangle_index] = Some(remap.segment_user_index);
                }
            }
            continue;
        }
        for triangle in &partition.triangles {
            let Some(global_triangle) =
                partition_triangle_to_global(*triangle, &partition.vertex_map)
            else {
                continue;
            };
            let Some(candidates) = triangle_lookup.get_mut(&sorted_triangle(global_triangle))
            else {
                continue;
            };
            if let Some(triangle_index) = candidates
                .iter()
                .copied()
                .find(|index| matches!(user_indices.get(*index), Some(None)))
            {
                user_indices[triangle_index] = Some(remap.segment_user_index);
            }
        }
    }

    coalesce_segment_specs(&user_indices)
}

fn partition_triangle_to_global(triangle: [u32; 3], vertex_map: &[u32]) -> Option<[u32; 3]> {
    Some([
        *vertex_map.get(triangle[0] as usize)?,
        *vertex_map.get(triangle[1] as usize)?,
        *vertex_map.get(triangle[2] as usize)?,
    ])
}

fn coalesce_segment_specs(user_indices: &[Option<u32>]) -> Vec<SegmentSpec> {
    let mut specs = Vec::new();
    let mut start = 0usize;
    let mut current = user_indices.first().copied().flatten().unwrap_or(32);

    for (index, user_index) in user_indices.iter().enumerate().skip(1) {
        let user_index = user_index.unwrap_or(32);
        if user_index == current {
            continue;
        }
        specs.push(SegmentSpec {
            triangle_start: start as u32,
            triangle_count: (index - start) as u32,
            user_index: current,
        });
        start = index;
        current = user_index;
    }
    specs.push(SegmentSpec {
        triangle_start: start as u32,
        triangle_count: (user_indices.len() - start) as u32,
        user_index: current,
    });
    specs
}

#[derive(Debug, Clone, Default)]
struct ShapeGeometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 3]>,
    bitangents: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    vertex_colors: Option<Vec<[f32; 4]>>,
    triangles: Vec<[u32; 3]>,
    bounding_sphere: Option<NifValue>,
}

impl ShapeGeometry {
    fn from_shape(nif: &NifFile, shape: &NifBlock) -> Self {
        if value_array(shape.get_field("Vertex Data")).is_empty() {
            if let Some(data) = legacy_geometry_data(nif, shape) {
                return Self::from_legacy_data(data);
            }
        }

        let mut geometry = Self {
            triangles: value_array(shape.get_field("Triangles"))
                .iter()
                .filter_map(triangle_from_value)
                .collect(),
            bounding_sphere: shape.get_field("Bounding Sphere").cloned(),
            ..Self::default()
        };
        geometry.set_vertex_entries(&value_array(shape.get_field("Vertex Data")));
        geometry
    }

    fn from_legacy_data(data: &NifBlock) -> Self {
        let positions = value_array(data.get_field("Vertices"))
            .iter()
            .filter_map(vec3_value)
            .collect::<Vec<_>>();
        let normals = value_array(data.get_field("Normals"))
            .iter()
            .filter_map(vec3_value)
            .collect::<Vec<_>>();
        let tangents = value_array(data.get_field("Tangents"))
            .iter()
            .filter_map(vec3_value)
            .collect::<Vec<_>>();
        let bitangents = value_array(data.get_field("Bitangents"))
            .iter()
            .filter_map(vec3_value)
            .collect::<Vec<_>>();
        let uvs = value_array(data.get_field("UV Sets"))
            .first()
            .and_then(|value| match value {
                NifValue::Array(items) => {
                    Some(items.iter().filter_map(uv_value).collect::<Vec<[f32; 2]>>())
                }
                _ => None,
            })
            .unwrap_or_default();
        let colors = value_array(data.get_field("Vertex Colors"))
            .iter()
            .filter_map(color4_value)
            .collect::<Vec<_>>();
        let triangles = value_array(data.get_field("Triangles"))
            .iter()
            .filter_map(triangle_from_value)
            .collect::<Vec<_>>();

        Self {
            positions,
            normals,
            tangents,
            bitangents,
            uvs,
            vertex_colors: (!colors.is_empty()).then_some(colors),
            triangles,
            bounding_sphere: data.get_field("Bounding Sphere").cloned(),
        }
    }

    fn complete_partition_geometry(
        &mut self,
        nif: &NifFile,
        shape: &NifBlock,
        parsed: &LegacySkin,
    ) {
        let mut partition_triangles_are_global = false;
        if let Some(partition) = parsed
            .skin_partition_block_id
            .and_then(|id| nif.get_block(id))
        {
            let entries = value_array(partition.get_field("Vertex Data"));
            partition_triangles_are_global = !entries.is_empty();
            if self.positions.is_empty() && !entries.is_empty() {
                self.set_vertex_entries(&entries);
            } else if shape.type_name == "BSDynamicTriShape" {
                let positions = value_array(shape.get_field("Vertices"))
                    .iter()
                    .filter_map(vec3_value)
                    .collect::<Vec<_>>();
                if !positions.is_empty() {
                    self.positions = positions;
                }
            }
        }

        if self.triangles.is_empty() {
            self.triangles = parsed
                .partitions
                .iter()
                .flat_map(|partition| {
                    partition.triangles.iter().filter_map(|triangle| {
                        if partition_triangles_are_global {
                            Some(*triangle)
                        } else {
                            partition_triangle_to_global(*triangle, &partition.vertex_map)
                        }
                    })
                })
                .collect();
        }
        if self.normals.len() != self.positions.len() {
            self.normals = recompute_normals(&self.positions, &self.triangles);
        }
    }

    fn set_vertex_entries(&mut self, vertex_entries: &[NifValue]) {
        self.positions.clear();
        self.normals.clear();
        self.tangents.clear();
        self.bitangents.clear();
        self.uvs.clear();
        let mut colors = Vec::with_capacity(vertex_entries.len());
        let mut has_colors = false;

        for entry in vertex_entries {
            let NifValue::Struct(fields) = entry else {
                continue;
            };
            self.positions.push(
                fields
                    .get("Vertex")
                    .and_then(vec3_value)
                    .unwrap_or([0.0, 0.0, 0.0]),
            );
            self.normals.push(
                fields
                    .get("Normal")
                    .and_then(vec3_value)
                    .unwrap_or([0.0, 0.0, 1.0]),
            );
            self.tangents.push(
                fields
                    .get("Tangent")
                    .and_then(vec3_value)
                    .unwrap_or([0.0, 0.0, 0.0]),
            );
            self.bitangents.push([
                value_f64(fields.get("Bitangent X")).unwrap_or(0.0) as f32,
                value_f64(fields.get("Bitangent Y")).unwrap_or(0.0) as f32,
                value_f64(fields.get("Bitangent Z")).unwrap_or(0.0) as f32,
            ]);
            self.uvs
                .push(fields.get("UV").and_then(uv_value).unwrap_or([0.0, 0.0]));
            if let Some(color) = fields.get("Vertex Colors").and_then(color4_value) {
                has_colors = true;
                colors.push(color);
            } else {
                colors.push([1.0, 1.0, 1.0, 1.0]);
            }
        }
        self.vertex_colors = has_colors.then_some(colors);
    }

    fn tangents_missing(&self) -> bool {
        self.tangents
            .iter()
            .chain(self.bitangents.iter())
            .all(|vector| vector.iter().all(|component| component.abs() <= 1e-8))
    }

    fn apply_transform(&mut self, transform: &SkinTransform) {
        if transform.is_identity() {
            return;
        }
        for position in &mut self.positions {
            *position = transform.transform_point(*position);
        }
        for normal in &mut self.normals {
            *normal = transform.transform_vector(*normal);
        }
        for tangent in &mut self.tangents {
            *tangent = transform.transform_vector(*tangent);
        }
        for bitangent in &mut self.bitangents {
            *bitangent = transform.transform_vector(*bitangent);
        }
        if let Some(NifValue::Struct(fields)) = &mut self.bounding_sphere {
            if let Some(center) = fields.get("Center").and_then(vec3_value) {
                fields.insert(
                    "Center".into(),
                    NifValue::Vec3(transform.transform_point(center)),
                );
            }
            if let Some(radius) = value_f64(fields.get("Radius")) {
                fields.insert(
                    "Radius".into(),
                    NifValue::Float((radius as f32 * transform.scale.abs()) as f64),
                );
            }
        }
    }
}

fn partition_triangles_use_global_vertices(nif: &NifFile, parsed: &LegacySkin) -> bool {
    parsed
        .skin_partition_block_id
        .and_then(|id| nif.get_block(id))
        .is_some_and(|partition| {
            matches!(partition.get_field("Vertex Data"), Some(NifValue::Array(entries)) if !entries.is_empty())
        })
}

fn recompute_normals(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0_f32; 3]; positions.len()];
    for triangle in triangles {
        let [a, b, c] = triangle.map(|index| index as usize);
        let (Some(a), Some(b), Some(c)) = (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        for index in [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ] {
            if let Some(normal) = normals.get_mut(index) {
                normal[0] += face[0];
                normal[1] += face[1];
                normal[2] += face[2];
            }
        }
    }
    for normal in &mut normals {
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if length > 1e-8 {
            normal[0] /= length;
            normal[1] /= length;
            normal[2] /= length;
        } else {
            *normal = [0.0, 0.0, 1.0];
        }
    }
    normals
}

fn legacy_geometry_data<'a>(nif: &'a NifFile, shape: &NifBlock) -> Option<&'a NifBlock> {
    let data_id = value_ref(shape.get_field("Data")).filter(|id| *id >= 0)? as usize;
    let data = nif.get_block(data_id)?;
    matches!(
        data.type_name.as_str(),
        "NiTriShapeData" | "NiTriStripsData"
    )
    .then_some(data)
}

fn legacy_geometry_data_ref(nif: &NifFile, shape_id: usize) -> Option<usize> {
    let shape = nif.get_block(shape_id)?;
    let data_id = value_ref(shape.get_field("Data")).filter(|id| *id >= 0)? as usize;
    let data = nif.get_block(data_id)?;
    matches!(
        data.type_name.as_str(),
        "NiTriShapeData" | "NiTriStripsData"
    )
    .then_some(data_id)
}

fn geometry_property_refs(nif: &NifFile, shape: &NifBlock) -> (i32, i32) {
    let mut shader_ref = value_ref(shape.get_field("Shader Property")).unwrap_or(-1);
    let mut alpha_ref = value_ref(shape.get_field("Alpha Property")).unwrap_or(-1);
    for prop_ref in ref_array(shape.get_field("Properties")) {
        let Some(prop) = (prop_ref >= 0)
            .then(|| nif.get_block(prop_ref as usize))
            .flatten()
        else {
            continue;
        };
        match prop.type_name.as_str() {
            "NiAlphaProperty" => alpha_ref = prop_ref,
            "TallGrassShaderProperty"
            | "BSShaderPPLightingProperty"
            | "BSLightingShaderProperty"
            | "BSEffectShaderProperty"
            | "Lighting30ShaderProperty" => shader_ref = prop_ref,
            _ => {}
        }
    }
    (shader_ref, alpha_ref)
}

fn extract_reference_skin(nif: &NifFile) -> ReferenceSkin {
    let mut positions = Vec::new();
    let mut influences = Vec::new();
    let mut bones = Vec::new();
    let mut bone_lookup = HashMap::new();

    for shape in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape"
        )
    }) {
        let local_bones = shape_bone_names(nif, shape);
        let mut local_to_global = Vec::new();
        for bone_name in local_bones {
            let global_index = match bone_lookup.get(&bone_name).copied() {
                Some(index) => index,
                None => {
                    let index = bones.len();
                    bones.push(bone_name.clone());
                    bone_lookup.insert(bone_name, index);
                    index
                }
            };
            local_to_global.push(global_index);
        }

        for entry in value_array(shape.get_field("Vertex Data")) {
            let NifValue::Struct(fields) = entry else {
                continue;
            };
            positions.push(
                fields
                    .get("Vertex")
                    .and_then(vec3_value)
                    .unwrap_or([0.0, 0.0, 0.0]),
            );
            let indices = numeric_array(fields.get("Bone Indices"));
            let weights = float_array(fields.get("Bone Weights"));
            let slots = indices
                .iter()
                .zip(weights.iter())
                .filter_map(|(local_index, weight)| {
                    let global_index = local_to_global.get(*local_index as usize).copied()?;
                    (*weight > 0.0).then_some((global_index, *weight))
                })
                .collect();
            influences.push(VertexInfluences { slots });
        }
    }

    (positions, influences, bones)
}

fn reference_bone_transforms(nif: &NifFile, bone_names: &[String]) -> Vec<SkinTransform> {
    let mut transforms = HashMap::<String, SkinTransform>::new();
    for shape in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape"
        )
    }) {
        let local_bones = shape_bone_names(nif, shape);
        let Some(skin_id) = value_ref(shape.get_field("Skin")).filter(|id| *id >= 0) else {
            continue;
        };
        let Some(data_id) = nif
            .get_block(skin_id as usize)
            .and_then(|skin| value_ref(skin.get_field("Data")))
            .filter(|id| *id >= 0)
        else {
            continue;
        };
        let bone_list = nif
            .get_block(data_id as usize)
            .map(|data| value_array(data.get_field("Bone List")))
            .unwrap_or_default();
        for (bone_name, bone_value) in local_bones.iter().zip(bone_list.iter()) {
            let NifValue::Struct(fields) = bone_value else {
                continue;
            };
            let rotation = fields
                .get("Rotation")
                .and_then(source::matrix33_value)
                .unwrap_or_else(|| SkinTransform::identity().rotation);
            transforms
                .entry(bone_name.to_ascii_lowercase())
                .or_insert(SkinTransform {
                    translation: fields
                        .get("Translation")
                        .and_then(vec3_value)
                        .unwrap_or([0.0; 3]),
                    rotation,
                    scale: value_f64(fields.get("Scale")).unwrap_or(1.0) as f32,
                });
        }
    }
    bone_names
        .iter()
        .map(|name| {
            transforms
                .get(&name.to_ascii_lowercase())
                .copied()
                .unwrap_or_default()
        })
        .collect()
}

fn shape_bone_names(nif: &NifFile, shape: &NifBlock) -> Vec<String> {
    let skin_ref = match shape
        .get_field("Skin")
        .or_else(|| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(id)) if *id >= 0 => *id as usize,
        _ => return Vec::new(),
    };
    let Some(skin) = nif.get_block(skin_ref) else {
        return Vec::new();
    };
    ref_array(skin.get_field("Bones"))
        .into_iter()
        .filter_map(|id| {
            (id >= 0).then(|| {
                nif.get_block(id as usize)
                    .and_then(|bone| bone.get_field("Name"))
                    .and_then(|value| match value {
                        NifValue::String(name) => Some(name.trim_end_matches('\0').to_string()),
                        _ => None,
                    })
                    .unwrap_or_default()
            })
        })
        .collect()
}

fn source_bone_refs(nif: &NifFile, instance_block_id: usize) -> Vec<i32> {
    nif.get_block(instance_block_id)
        .map(|instance| ref_array(instance.get_field("Bones")))
        .unwrap_or_default()
}

fn find_node_by_name(nif: &NifFile, name: &str) -> Option<usize> {
    nif.blocks
        .iter()
        .find(|block| {
            is_node_type(&block.type_name)
                && matches!(block.get_field("Name"), Some(NifValue::String(value)) if value.trim_end_matches('\0') == name)
        })
        .map(|block| block.block_id)
}

fn find_node_by_name_case_insensitive(nif: &NifFile, name: &str) -> Option<usize> {
    nif.blocks
        .iter()
        .find(|block| {
            is_node_type(&block.type_name)
                && matches!(block.get_field("Name"), Some(NifValue::String(value)) if value.trim_end_matches('\0').eq_ignore_ascii_case(name))
        })
        .map(|block| block.block_id)
}

fn distance_squared(left: [f32; 3], right: [f32; 3]) -> f32 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    let dz = left[2] - right[2];
    dx * dx + dy * dy + dz * dz
}

fn is_node_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "NiNode" | "BSFadeNode" | "BSLeafAnimNode" | "BSOrderedNode" | "NiBillboardNode"
    )
}

fn bone_bounds(
    bone_index: usize,
    positions: &[[f32; 3]],
    influences: &[VertexInfluences],
) -> ([f32; 3], f32) {
    let mut weighted_positions = Vec::new();
    for (vertex_index, influence) in influences.iter().enumerate() {
        let weight: f32 = influence
            .slots
            .iter()
            .filter(|(index, _)| *index == bone_index)
            .map(|(_, weight)| *weight)
            .sum();
        if weight > 0.0 {
            if let Some(position) = positions.get(vertex_index).copied() {
                weighted_positions.push(position);
            }
        }
    }
    if weighted_positions.is_empty() {
        return ([0.0, 0.0, 0.0], 0.0);
    }
    let mut center = [0.0_f32; 3];
    for position in &weighted_positions {
        center[0] += position[0];
        center[1] += position[1];
        center[2] += position[2];
    }
    let count = weighted_positions.len() as f32;
    center[0] /= count;
    center[1] /= count;
    center[2] /= count;

    let radius = weighted_positions
        .iter()
        .map(|position| distance(center, *position))
        .fold(0.0_f32, f32::max);
    (center, radius)
}

fn bone_data_entry((center, radius): ([f32; 3], f32), transform: SkinTransform) -> NifValue {
    let mut bound = IndexMap::new();
    bound.insert("Center".into(), NifValue::Vec3(center));
    bound.insert("Radius".into(), NifValue::Float(radius as f64));

    let mut fields = IndexMap::new();
    fields.insert("Bounding Sphere".into(), NifValue::Struct(bound));
    fields.insert("Rotation".into(), NifValue::Matrix33(transform.rotation));
    fields.insert("Translation".into(), NifValue::Vec3(transform.translation));
    fields.insert("Scale".into(), NifValue::Float(transform.scale as f64));
    NifValue::Struct(fields)
}

fn sphere_value(value: &NifValue) -> Option<([f32; 3], f32)> {
    let NifValue::Struct(fields) = value else {
        return None;
    };
    let center = fields.get("Center").and_then(vec3_value)?;
    let radius = value_f64(fields.get("Radius"))? as f32;
    (center.iter().all(|value| value.is_finite()) && radius.is_finite() && radius >= 0.0)
        .then_some((center, radius))
}

fn mark_shader_skinned(nif: &mut NifFile, shape_id: usize) {
    let shader_ref = nif
        .get_block(shape_id)
        .and_then(|shape| value_ref(shape.get_field("Shader Property")))
        .filter(|id| *id >= 0);
    let Some(shader_ref) = shader_ref else {
        return;
    };
    let Some(shader) = nif.blocks.get_mut(shader_ref as usize) else {
        return;
    };
    if !matches!(
        shader.type_name.as_str(),
        "BSLightingShaderProperty" | "BSEffectShaderProperty"
    ) {
        return;
    }
    let flags = value_u64(shader.get_field("Shader Flags 1")).unwrap_or(0);
    let flags = flags | 0x02;
    shader.set_field("Shader Flags 1", NifValue::UInt(flags));
    if shader.fields.contains_key("Shader Flags 1:FO4") {
        shader
            .fields
            .insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(flags));
    }
}

fn data_size(shape: &NifBlock) -> i64 {
    let stride = shape
        .get_field("Vertex Desc")
        .map(NifValue::as_i64)
        .unwrap_or(0)
        & 0xF;
    let vertices = shape
        .get_field("Num Vertices")
        .map(NifValue::as_i64)
        .unwrap_or(0)
        .max(0);
    let triangles = shape
        .get_field("Num Triangles")
        .map(NifValue::as_i64)
        .unwrap_or(0)
        .max(0);
    stride * vertices * 4 + triangles * 6
}

fn merge_slot(slots: &mut Vec<(usize, f32)>, bone_index: usize, weight: f32) {
    if let Some((_, existing_weight)) = slots.iter_mut().find(|(index, _)| *index == bone_index) {
        *existing_weight += weight;
    } else {
        slots.push((bone_index, weight));
    }
}

fn normalize_top_four(slots: &mut Vec<(usize, f32)>) {
    slots.retain(|(_, weight)| *weight > 0.0);
    slots.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    slots.truncate(4);
    let total: f32 = slots.iter().map(|(_, weight)| *weight).sum();
    if total > 0.0 {
        for (_, weight) in slots {
            *weight /= total;
        }
    }
}

fn triangle_value(triangle: [u32; 3]) -> NifValue {
    let mut fields = IndexMap::new();
    fields.insert("v1".into(), NifValue::UInt(triangle[0] as u64));
    fields.insert("v2".into(), NifValue::UInt(triangle[1] as u64));
    fields.insert("v3".into(), NifValue::UInt(triangle[2] as u64));
    NifValue::Struct(fields)
}

fn sorted_triangle(mut triangle: [u32; 3]) -> [u32; 3] {
    triangle.sort_unstable();
    triangle
}

fn triangle_from_value(value: &NifValue) -> Option<[u32; 3]> {
    let NifValue::Struct(fields) = value else {
        return None;
    };
    Some([
        fields.get("v1").map(NifValue::as_i64)? as u32,
        fields.get("v2").map(NifValue::as_i64)? as u32,
        fields.get("v3").map(NifValue::as_i64)? as u32,
    ])
}

fn value_array(value: Option<&NifValue>) -> Vec<NifValue> {
    match value {
        Some(NifValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn ref_array(value: Option<&NifValue>) -> Vec<i32> {
    value_array(value)
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .collect()
}

fn numeric_array(value: Option<&NifValue>) -> Vec<u32> {
    value_array(value)
        .iter()
        .map(|value| value.as_i64().max(0) as u32)
        .collect()
}

fn float_array(value: Option<&NifValue>) -> Vec<f32> {
    value_array(value)
        .iter()
        .filter_map(|value| match value {
            NifValue::Float(number) => Some(*number as f32),
            NifValue::Int(number) => Some(*number as f32),
            NifValue::UInt(number) => Some(*number as f32),
            _ => None,
        })
        .collect()
}

fn value_ref(value: Option<&NifValue>) -> Option<i32> {
    match value? {
        NifValue::Ref(id) => Some(*id),
        NifValue::Int(id) => Some(*id as i32),
        NifValue::UInt(id) => Some(*id as i32),
        _ => None,
    }
}

fn value_u64(value: Option<&NifValue>) -> Option<u64> {
    match value? {
        NifValue::UInt(value) => Some(*value),
        NifValue::Int(value) if *value >= 0 => Some(*value as u64),
        NifValue::Ref(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

fn vec3_value(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("x")).unwrap_or(0.0) as f32,
            value_f64(fields.get("y")).unwrap_or(0.0) as f32,
            value_f64(fields.get("z")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

fn uv_value(value: &NifValue) -> Option<[f32; 2]> {
    match value {
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("u")).unwrap_or(0.0) as f32,
            value_f64(fields.get("v")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

fn color4_value(value: &NifValue) -> Option<[f32; 4]> {
    match value {
        NifValue::Color4(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            (value_f64(fields.get("r")).unwrap_or(255.0) / 255.0) as f32,
            (value_f64(fields.get("g")).unwrap_or(255.0) / 255.0) as f32,
            (value_f64(fields.get("b")).unwrap_or(255.0) / 255.0) as f32,
            (value_f64(fields.get("a")).unwrap_or(255.0) / 255.0) as f32,
        ]),
        _ => None,
    }
}

fn distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    let dz = left[2] - right[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_partition_geometry_keeps_global_triangle_indices() {
        let mut nif = NifFile::new("skyrimse");
        let mut partition_fields = IndexMap::new();
        partition_fields.insert(
            "Vertex Data".to_string(),
            NifValue::Array(
                [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
                    .into_iter()
                    .map(|uv| {
                        let mut tex_coord = IndexMap::new();
                        tex_coord.insert("u".to_string(), NifValue::Float(uv[0]));
                        tex_coord.insert("v".to_string(), NifValue::Float(uv[1]));
                        let mut vertex = IndexMap::new();
                        vertex.insert("UV".to_string(), NifValue::Struct(tex_coord));
                        NifValue::Struct(vertex)
                    })
                    .collect(),
            ),
        );
        let partition_id = nif.add_block("NiSkinPartition", Some(partition_fields));
        let mut shape = NifBlock::new(1, "BSDynamicTriShape");
        shape.set_field(
            "Vertices",
            NifValue::Array(
                [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                    .into_iter()
                    .map(NifValue::Vec3)
                    .collect(),
            ),
        );
        let parsed = LegacySkin {
            kind: source::LegacySkinKind::NonArmor,
            instance_block_id: 0,
            skin_data_block_id: 0,
            skin_partition_block_id: Some(partition_id),
            skeleton_root: None,
            bones: Vec::new(),
            skin_transform: SkinTransform::identity(),
            data_influences: Vec::new(),
            bone_transforms: Vec::new(),
            bone_bounds: Vec::new(),
            partitions: vec![source::LegacyPartition {
                body_part: 0,
                vertex_map: vec![2, 0, 1],
                influences: Vec::new(),
                bones: Vec::new(),
                triangles: vec![[0, 1, 2]],
            }],
        };

        let mut geometry = ShapeGeometry::from_shape(&nif, &shape);
        geometry.complete_partition_geometry(&nif, &shape, &parsed);

        assert_eq!(geometry.positions.len(), 3);
        assert_eq!(geometry.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        assert_eq!(geometry.triangles, vec![[0, 1, 2]]);
        assert_eq!(geometry.normals, vec![[0.0, 0.0, 1.0]; 3]);
    }

    #[test]
    fn source_rig_influences_preserve_palette_lane_order_and_vertex_map() {
        let parsed = LegacySkin {
            kind: source::LegacySkinKind::NonArmor,
            instance_block_id: 0,
            skin_data_block_id: 0,
            skin_partition_block_id: None,
            skeleton_root: None,
            bones: (0..4)
                .map(|index| bone_remap::BoneEntry {
                    name: format!("Bone{index}"),
                    parent: index as i32 - 1,
                })
                .collect(),
            skin_transform: SkinTransform::identity(),
            data_influences: Vec::new(),
            bone_transforms: vec![SkinTransform::identity(); 4],
            bone_bounds: Vec::new(),
            partitions: vec![source::LegacyPartition {
                body_part: 0,
                vertex_map: vec![1, 0],
                influences: vec![
                    vec![(0, 0.25), (1, 0.75), (1, 0.0), (0, 0.0)],
                    vec![(1, 0.2), (0, 0.8), (0, 0.0), (1, 0.0)],
                ],
                bones: vec![3, 1],
                triangles: Vec::new(),
            }],
        };

        let influences = source_rig_influences(&parsed, 2).expect("valid source lanes");

        assert_eq!(
            influences[0].slots,
            vec![(1, 0.2), (3, 0.8), (3, 0.0), (1, 0.0)]
        );
        assert_eq!(
            influences[1].slots,
            vec![(3, 0.25), (1, 0.75), (1, 0.0), (3, 0.0)]
        );
    }

    #[test]
    fn source_rig_influences_accept_duplicate_partition_vertices_only_for_equivalent_lanes() {
        let mut parsed = LegacySkin {
            kind: source::LegacySkinKind::Armor,
            instance_block_id: 0,
            skin_data_block_id: 0,
            skin_partition_block_id: None,
            skeleton_root: None,
            bones: (0..4)
                .map(|index| bone_remap::BoneEntry {
                    name: format!("Bone{index}"),
                    parent: index as i32 - 1,
                })
                .collect(),
            skin_transform: SkinTransform::identity(),
            data_influences: Vec::new(),
            bone_transforms: vec![SkinTransform::identity(); 4],
            bone_bounds: Vec::new(),
            partitions: vec![
                source::LegacyPartition {
                    body_part: 0,
                    vertex_map: vec![0],
                    influences: vec![vec![(0, 0.7), (1, 0.3), (2, 0.0), (3, 0.0)]],
                    bones: vec![0, 1, 2, 3],
                    triangles: Vec::new(),
                },
                source::LegacyPartition {
                    body_part: 1,
                    vertex_map: vec![0],
                    influences: vec![vec![(0, 0.7), (1, 0.3), (1, 0.0), (1, 0.0)]],
                    bones: vec![0, 1],
                    triangles: Vec::new(),
                },
            ],
        };

        let influences = source_rig_influences(&parsed, 1).expect("equivalent duplicate lanes");
        assert_eq!(
            influences[0].slots,
            vec![(0, 0.7), (1, 0.3), (2, 0.0), (3, 0.0)]
        );

        parsed.partitions[1].influences[0][1].1 = 0.2;
        assert!(source_rig_influences(&parsed, 1).is_none());
    }

    #[test]
    fn source_rig_segments_collapse_to_root_without_changing_humanoid_translation() {
        let parsed = LegacySkin {
            kind: source::LegacySkinKind::Armor,
            instance_block_id: 0,
            skin_data_block_id: 0,
            skin_partition_block_id: None,
            skeleton_root: None,
            bones: Vec::new(),
            skin_transform: SkinTransform::identity(),
            data_influences: Vec::new(),
            bone_transforms: Vec::new(),
            bone_bounds: Vec::new(),
            partitions: vec![source::LegacyPartition {
                body_part: 2,
                vertex_map: vec![0, 1, 2],
                influences: Vec::new(),
                bones: Vec::new(),
                triangles: vec![[0, 1, 2]],
            }],
        };
        let triangles = [[0, 1, 2]];
        let nonempty_segment_indices = |shape: &NifBlock| match shape.get_field("Segment") {
            Some(NifValue::Array(segments)) => segments
                .iter()
                .enumerate()
                .filter_map(|(index, segment)| match segment {
                    NifValue::Struct(fields)
                        if fields
                            .get("Num Primitives")
                            .is_some_and(|count| count.as_i64() > 0) =>
                    {
                        Some(index)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let user_indices = |shape: &NifBlock| match shape.get_field("Segment") {
            Some(NifValue::Array(segments)) => segments
                .iter()
                .filter_map(|segment| match segment {
                    NifValue::Struct(fields) => fields.get("User Index").map(NifValue::as_i64),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };

        for source_game in ["skyrimse", "fnv", "fo3"] {
            let mut shape = NifBlock::new(0, "BSSubIndexTriShape");
            write_segments(
                &mut shape,
                &parsed,
                &triangles,
                source_game,
                LegacySkinPolicy::PreserveSourceRig,
            );
            assert_eq!(
                shape.get_field("Num Segments").map(NifValue::as_i64),
                Some(33)
            );
            assert_eq!(
                shape.get_field("Total Segments").map(NifValue::as_i64),
                Some(33)
            );
            assert_eq!(nonempty_segment_indices(&shape), vec![32]);
        }

        let mut translated = NifBlock::new(0, "BSSubIndexTriShape");
        write_segments(
            &mut translated,
            &parsed,
            &triangles,
            "fnv",
            LegacySkinPolicy::TranslateSkeleton,
        );
        assert_eq!(user_indices(&translated), vec![34]);

        let mut skyrim_parsed = parsed.clone();
        skyrim_parsed.partitions[0].body_part = 32;
        let mut skyrim_translated = NifBlock::new(0, "BSSubIndexTriShape");
        write_segments(
            &mut skyrim_translated,
            &skyrim_parsed,
            &triangles,
            "skyrimse",
            LegacySkinPolicy::TranslateSkeleton,
        );
        assert_eq!(user_indices(&skyrim_translated), vec![33]);
    }
}
