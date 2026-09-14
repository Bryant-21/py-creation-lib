use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::collision::compound::{CompoundChild, CompoundChildKind};
use crate::collision::compressed_mesh::BuildOptions;
use crate::collision::constraints::{GraftCinfo, GraftedConstraints};
use crate::collision::multi_body::{
    BodyMeta, MultiBodyShape, build_fo4_multi_body_collision_with_constraints,
};
use crate::hkx::HkxFile;
use crate::hkx::descriptors::{DescriptorRegistry, MemberTemplate};
use crate::hkx::model::{HkxMember, HkxObject};
use crate::hkx::types::{HkxType, HkxValue, half_to_f32};

const FO4_CLASS_VERSION: u32 = 11;
const FO4_CONTENTS_VERSION: &str = "hk_2014.1.0-r1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QsTransformIr {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl QsTransformIr {
    pub fn identity() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigBoneIr {
    pub name: String,
    pub parent: Option<usize>,
    pub reference_pose: QsTransformIr,
    pub lock_translation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigSkeletonIr {
    pub name: String,
    pub bones: Vec<RigBoneIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvexHullIr {
    pub vertices: Vec<[f32; 3]>,
    pub planes: Vec<[f32; 4]>,
    pub convex_radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RagdollShapeIr {
    ConvexHull(ConvexHullIr),
    Compound {
        children: Vec<RagdollShapeIr>,
    },
    Capsule {
        vertex_a: [f32; 3],
        vertex_b: [f32; 3],
        radius: f32,
    },
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    Box {
        half_extents: [f32; 3],
        convex_radius: f32,
    },
}

impl RagdollShapeIr {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::ConvexHull(_) => "convex_hull",
            Self::Compound { .. } => "compound",
            Self::Capsule { .. } => "capsule",
            Self::Sphere { .. } => "sphere",
            Self::Box { .. } => "box",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MassPropertiesIr {
    pub mass: f32,
    pub center_of_mass: [f32; 3],
    pub inertia_diagonal: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigidBodyIr {
    pub name: String,
    pub ragdoll_bone: String,
    pub shape: RagdollShapeIr,
    pub world_from_body: QsTransformIr,
    pub mass_properties: MassPropertiesIr,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub collision_filter_info: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitedHingeIr {
    pub frame_a: QsTransformIr,
    pub frame_b: QsTransformIr,
    pub min_angle: f32,
    pub max_angle: f32,
    pub max_friction_torque: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagdollLimitsIr {
    pub frame_a: QsTransformIr,
    pub frame_b: QsTransformIr,
    pub cone_limit: f32,
    pub plane_min: f32,
    pub plane_max: f32,
    pub twist_min: f32,
    pub twist_max: f32,
    pub max_friction_torque: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HingeIr {
    pub frame_a: QsTransformIr,
    pub frame_b: QsTransformIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrismaticIr {
    pub frame_a: QsTransformIr,
    pub frame_b: QsTransformIr,
    pub min_distance: f32,
    pub max_distance: f32,
    pub max_friction_force: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintKindIr {
    LimitedHinge(LimitedHingeIr),
    Hinge(HingeIr),
    Prismatic(PrismaticIr),
    Breakable {
        inner: Box<ConstraintKindIr>,
        threshold: f32,
    },
    Ragdoll(RagdollLimitsIr),
    Fixed {
        frame_a: QsTransformIr,
        frame_b: QsTransformIr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintIr {
    pub name: String,
    pub body_a: String,
    pub body_b: String,
    pub kind: ConstraintKindIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneMappingIr {
    pub ragdoll_bone: String,
    pub animation_bone: String,
    pub ragdoll_from_animation: QsTransformIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatureRagdollIr {
    pub name: String,
    pub animation_skeleton: RigSkeletonIr,
    pub ragdoll_skeleton: RigSkeletonIr,
    pub bodies: Vec<RigidBodyIr>,
    pub constraints: Vec<ConstraintIr>,
    pub mappings: Vec<BoneMappingIr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedCreatureRagdoll {
    pub binary_data: Vec<u8>,
    pub animation_bone_targets: Vec<String>,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CreatureRagdollError {
    #[error("{skeleton} skeleton has no bones")]
    EmptySkeleton { skeleton: String },
    #[error("{skeleton} skeleton has invalid root count {roots}")]
    InvalidRootCount { skeleton: String, roots: usize },
    #[error("{skeleton} skeleton bone {bone} has invalid parent index {parent}")]
    InvalidParent {
        skeleton: String,
        bone: String,
        parent: usize,
    },
    #[error("{skeleton} skeleton contains duplicate bone {bone}")]
    DuplicateBone { skeleton: String, bone: String },
    #[error("duplicate rigid body name {body}")]
    DuplicateBody { body: String },
    #[error("ragdoll bone {bone} is linked to more than one rigid body")]
    DuplicateBodyBone { bone: String },
    #[error("ragdoll has no rigid bodies")]
    EmptyBodies,
    #[error("rigid body {body} references missing ragdoll bone {bone}")]
    BodyBoneMissing { body: String, bone: String },
    #[error("rigid body {body} has no mapper path to the animation skeleton")]
    BodyMappingMissing { body: String },
    #[error("bone mapping references missing {skeleton} bone {bone}")]
    MappingBoneMissing { skeleton: String, bone: String },
    #[error("duplicate mapping for ragdoll bone {bone}")]
    DuplicateMapping { bone: String },
    #[error("constraint {constraint} references missing rigid body {body}")]
    ConstraintBodyMissing { constraint: String, body: String },
    #[error("constraint {constraint} connects rigid body {body} to itself")]
    SelfConstraint { constraint: String, body: String },
    #[error("{path} contains a non-finite value")]
    NonFinite { path: String },
    #[error("{path} must be greater than zero")]
    NonPositive { path: String },
    #[error("{path} must not be negative")]
    Negative { path: String },
    #[error("constraint {constraint} has minimum angle greater than maximum angle")]
    InvalidAngleRange { constraint: String },
    #[error("convex hull for rigid body {body} needs at least four vertices and four planes")]
    InvalidConvexHull { body: String },
    #[error("convex hull plane {plane} for rigid body {body} has a zero normal")]
    InvalidPlane { body: String, plane: usize },
    #[error("primitive convexification needs at least six radial segments; found {segments}")]
    InvalidConvexificationSegments { segments: usize },
    #[error("could not convexify {shape} shape for rigid body {body}: {reason}")]
    PrimitiveConvexification {
        body: String,
        shape: String,
        reason: String,
    },
    #[error(
        "FO4 reconstruction for {shape} shape on rigid body {body} is not evidenced by the target descriptor corpus"
    )]
    UnsupportedTargetShape { body: String, shape: String },
    #[error(
        "FO4 reconstruction for {kind} constraint {constraint} is not evidenced by the target descriptor corpus"
    )]
    UnsupportedTargetConstraint { constraint: String, kind: String },
    #[error("FO4 descriptor unavailable for {class_name}")]
    DescriptorMissing { class_name: String },
    #[error("FO4 descriptor {class_name} has no member {member}")]
    DescriptorMemberMissing { class_name: String, member: String },
    #[error("FO4 descriptor error for {class_name}: {reason}")]
    Descriptor { class_name: String, reason: String },
    #[error("invalid descriptor signature {signature} for {class_name}")]
    InvalidSignature {
        class_name: String,
        signature: String,
    },
    #[error("target object {object} contains out-of-range pointer {target}")]
    PointerOutOfRange { object: usize, target: usize },
    #[error("target object {object} contains an unresolved pointer {target}")]
    PendingPointer { object: usize, target: String },
    #[error("FO4 ragdoll packfile could not be reread: {reason}")]
    PackfileReread { reason: String },
    #[error("FO4 embedded ragdoll could not be built: {reason}")]
    EmbeddedBuild { reason: String },
    #[error("FO4 ragdoll reread contract failed: {reason}")]
    RereadContract { reason: String },
    #[error(
        "unsupported Skyrim ragdoll source: classversion {class_version}, contents {contents_version}, pointer size {pointer_size}"
    )]
    UnsupportedSkyrimSource {
        class_version: u32,
        contents_version: String,
        pointer_size: u8,
    },
    #[error("Skyrim source expected {expected} {class_name} objects, found {actual}")]
    SkyrimSourceClassCount {
        class_name: String,
        expected: usize,
        actual: usize,
    },
    #[error("Skyrim source {kind} fixup missing at absolute offset {offset}")]
    SkyrimSourceFixupMissing { kind: String, offset: usize },
    #[error("Skyrim source pointer at absolute offset {offset} does not target a decoded object")]
    SkyrimSourceObjectMissing { offset: usize },
    #[error("Skyrim source layout error in {context}: {reason}")]
    SkyrimSourceLayout { context: String, reason: String },
    #[error("Skyrim source has no unique ragdoll-to-animation skeleton mapper")]
    SkyrimSourceMapperMissing,
    #[error("Skyrim source bone/body map is not a complete one-to-one closure: {reason}")]
    SkyrimSourceBoneBodyMap { reason: String },
    #[error("Skyrim source rigid body {body} uses unsupported shape {class_name}")]
    UnsupportedSkyrimSourceShape { body: String, class_name: String },
    #[error("Skyrim source constraint {constraint} uses unsupported data {class_name}")]
    UnsupportedSkyrimSourceConstraint {
        constraint: String,
        class_name: String,
    },
}

pub fn validate_creature_ragdoll_ir(ir: &CreatureRagdollIr) -> Result<(), CreatureRagdollError> {
    let animation_bones = validate_skeleton("animation", &ir.animation_skeleton, false)?;
    let ragdoll_bones = validate_skeleton("ragdoll", &ir.ragdoll_skeleton, true)?;

    let mut mapped_ragdoll_bones = HashSet::new();
    for (index, mapping) in ir.mappings.iter().enumerate() {
        if !ragdoll_bones.contains_key(mapping.ragdoll_bone.as_str()) {
            return Err(CreatureRagdollError::MappingBoneMissing {
                skeleton: "ragdoll".to_string(),
                bone: mapping.ragdoll_bone.clone(),
            });
        }
        if !animation_bones.contains_key(mapping.animation_bone.as_str()) {
            return Err(CreatureRagdollError::MappingBoneMissing {
                skeleton: "animation".to_string(),
                bone: mapping.animation_bone.clone(),
            });
        }
        if !mapped_ragdoll_bones.insert(mapping.ragdoll_bone.as_str()) {
            return Err(CreatureRagdollError::DuplicateMapping {
                bone: mapping.ragdoll_bone.clone(),
            });
        }
        validate_qs_transform(
            &format!("mappings[{index}].ragdoll_from_animation"),
            &mapping.ragdoll_from_animation,
        )?;
    }

    let mut body_names = HashSet::new();
    let mut body_bones = HashSet::new();
    if ir.bodies.is_empty() {
        return Err(CreatureRagdollError::EmptyBodies);
    }
    for body in &ir.bodies {
        if !body_names.insert(body.name.as_str()) {
            return Err(CreatureRagdollError::DuplicateBody {
                body: body.name.clone(),
            });
        }
        if !ragdoll_bones.contains_key(body.ragdoll_bone.as_str()) {
            return Err(CreatureRagdollError::BodyBoneMissing {
                body: body.name.clone(),
                bone: body.ragdoll_bone.clone(),
            });
        }
        if !body_bones.insert(body.ragdoll_bone.as_str()) {
            return Err(CreatureRagdollError::DuplicateBodyBone {
                bone: body.ragdoll_bone.clone(),
            });
        }
        if !mapped_ragdoll_bones.contains(body.ragdoll_bone.as_str()) {
            return Err(CreatureRagdollError::BodyMappingMissing {
                body: body.name.clone(),
            });
        }
        validate_body(body)?;
    }

    for constraint in &ir.constraints {
        for body in [&constraint.body_a, &constraint.body_b] {
            if !body_names.contains(body.as_str()) {
                return Err(CreatureRagdollError::ConstraintBodyMissing {
                    constraint: constraint.name.clone(),
                    body: body.clone(),
                });
            }
        }
        if constraint.body_a == constraint.body_b {
            return Err(CreatureRagdollError::SelfConstraint {
                constraint: constraint.name.clone(),
                body: constraint.body_a.clone(),
            });
        }
        validate_constraint(constraint)?;
    }
    Ok(())
}

pub fn reconstruct_fo4_creature_ragdoll(
    ir: &CreatureRagdollIr,
) -> Result<HkxFile, CreatureRagdollError> {
    validate_creature_ragdoll_ir(ir)?;
    reject_unsupported_target_types(ir)?;

    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    let root_signature = descriptor_signature(&mut registry, "hkRootLevelContainer")?;
    let mut objects = vec![HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: root_signature,
        class_name: "hkRootLevelContainer".to_string(),
        members: Vec::new(),
    }];

    let animation_skeleton_index =
        push_skeleton(&mut registry, &mut objects, &ir.animation_skeleton)?;
    let ragdoll_skeleton_index = push_skeleton(&mut registry, &mut objects, &ir.ragdoll_skeleton)?;
    let animation_container_index = push_target_object(
        &mut registry,
        &mut objects,
        "hkaAnimationContainer",
        vec![
            member(
                "skeletons",
                HkxValue::Array(vec![
                    HkxValue::Pointer(Some(animation_skeleton_index)),
                    HkxValue::Pointer(Some(ragdoll_skeleton_index)),
                ]),
            ),
            member("animations", HkxValue::Array(Vec::new())),
            member("bindings", HkxValue::Array(Vec::new())),
            member("attachments", HkxValue::Array(Vec::new())),
            member("skins", HkxValue::Array(Vec::new())),
        ],
    )?;

    let animation_bone_indices = bone_indices(&ir.animation_skeleton);
    let ragdoll_bone_indices = bone_indices(&ir.ragdoll_skeleton);
    let ragdoll_to_animation_mapper = push_mapper(
        &mut registry,
        &mut objects,
        ragdoll_skeleton_index,
        animation_skeleton_index,
        &ir.mappings,
        &ragdoll_bone_indices,
        &animation_bone_indices,
        false,
    )?;
    let animation_to_ragdoll_mapper = push_mapper(
        &mut registry,
        &mut objects,
        animation_skeleton_index,
        ragdoll_skeleton_index,
        &ir.mappings,
        &ragdoll_bone_indices,
        &animation_bone_indices,
        true,
    )?;

    let mut body_object_indices = Vec::with_capacity(ir.bodies.len());
    let mut body_index_by_name = HashMap::new();
    for (body_index, body) in ir.bodies.iter().enumerate() {
        let shape_index = push_ragdoll_shape(&mut registry, &mut objects, &body.shape)?;
        let object_index = push_rigid_body(&mut registry, &mut objects, body, shape_index)?;
        body_object_indices.push(object_index);
        body_index_by_name.insert(body.name.as_str(), (body_index, object_index));
    }

    let mut ragdoll_constraint_indices = Vec::with_capacity(ir.constraints.len());
    let mut physics_constraint_indices = Vec::with_capacity(ir.constraints.len());
    for constraint in &ir.constraints {
        let data_index = push_constraint_data(&mut registry, &mut objects, constraint)?;
        let body_a_index = body_index_by_name[constraint.body_a.as_str()].1;
        let body_b_index = body_index_by_name[constraint.body_b.as_str()].1;
        ragdoll_constraint_indices.push(push_constraint_instance(
            &mut registry,
            &mut objects,
            constraint,
            data_index,
            body_a_index,
            body_b_index,
        )?);
        physics_constraint_indices.push(push_constraint_instance(
            &mut registry,
            &mut objects,
            constraint,
            data_index,
            body_a_index,
            body_b_index,
        )?);
    }

    let mut bone_to_rigid_body = vec![HkxValue::I32(-1); ir.ragdoll_skeleton.bones.len()];
    for (body_index, body) in ir.bodies.iter().enumerate() {
        let bone_index = ragdoll_bone_indices[body.ragdoll_bone.as_str()];
        bone_to_rigid_body[bone_index] = HkxValue::I32(body_index as i32);
    }
    let ragdoll_instance_index = push_target_object(
        &mut registry,
        &mut objects,
        "hkaRagdollInstance",
        vec![
            member("rigidBodies", pointer_array(&body_object_indices)),
            member("constraints", pointer_array(&ragdoll_constraint_indices)),
            member("boneToRigidBodyMap", HkxValue::Array(bone_to_rigid_body)),
            member("skeleton", HkxValue::Pointer(Some(ragdoll_skeleton_index))),
        ],
    )?;

    let physics_system_index = push_target_object(
        &mut registry,
        &mut objects,
        "hkpPhysicsSystem",
        vec![
            member("rigidBodies", pointer_array(&body_object_indices)),
            member("constraints", pointer_array(&physics_constraint_indices)),
            member("actions", HkxValue::Array(Vec::new())),
            member("phantoms", HkxValue::Array(Vec::new())),
            string_member("name", &ir.name),
            member("userData", HkxValue::U64(0)),
            member("active", HkxValue::Bool(true)),
        ],
    )?;
    let physics_data_index = push_target_object(
        &mut registry,
        &mut objects,
        "hkpPhysicsData",
        vec![
            member("worldCinfo", HkxValue::Pointer(None)),
            member(
                "systems",
                HkxValue::Array(vec![HkxValue::Pointer(Some(physics_system_index))]),
            ),
        ],
    )?;

    objects[0].members = vec![member(
        "namedVariants",
        HkxValue::Array(vec![
            named_variant(
                "Animation Container",
                "hkaAnimationContainer",
                animation_container_index,
            ),
            named_variant("Physics Data", "hkpPhysicsData", physics_data_index),
            named_variant(
                "RagdollInstance",
                "hkaRagdollInstance",
                ragdoll_instance_index,
            ),
            named_variant(
                "SkeletonMapper",
                "hkaSkeletonMapper",
                animation_to_ragdoll_mapper,
            ),
            named_variant(
                "SkeletonMapper",
                "hkaSkeletonMapper",
                ragdoll_to_animation_mapper,
            ),
        ]),
    )];

    let file = HkxFile::from_tagxml(FO4_CLASS_VERSION, FO4_CONTENTS_VERSION, objects);
    validate_fo4_creature_ragdoll_file(&file, ir.bodies.len(), ir.constraints.len())?;
    Ok(file)
}

pub fn lower_primitive_shapes_to_fo4_convex_hulls(
    ir: &CreatureRagdollIr,
    radial_segments: usize,
) -> Result<CreatureRagdollIr, CreatureRagdollError> {
    if radial_segments < 6 {
        return Err(CreatureRagdollError::InvalidConvexificationSegments {
            segments: radial_segments,
        });
    }
    validate_creature_ragdoll_ir(ir)?;
    let mut lowered = ir.clone();
    for body in &mut lowered.bodies {
        body.shape = lower_shape_to_fo4_convex_hulls(&body.name, &body.shape, radial_segments)?;
    }
    validate_creature_ragdoll_ir(&lowered)?;
    Ok(lowered)
}

fn lower_shape_to_fo4_convex_hulls(
    body_name: &str,
    shape: &RagdollShapeIr,
    radial_segments: usize,
) -> Result<RagdollShapeIr, CreatureRagdollError> {
    Ok(match shape {
        RagdollShapeIr::ConvexHull(hull) => RagdollShapeIr::ConvexHull(hull.clone()),
        RagdollShapeIr::Compound { children } => RagdollShapeIr::Compound {
            children: children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    lower_shape_to_fo4_convex_hulls(
                        &format!("{body_name}.child[{index}]"),
                        child,
                        radial_segments,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        },
        RagdollShapeIr::Box {
            half_extents,
            convex_radius,
        } => RagdollShapeIr::ConvexHull(box_hull(*half_extents, *convex_radius)),
        RagdollShapeIr::Sphere { center, radius } => {
            RagdollShapeIr::ConvexHull(sampled_primitive_hull(
                body_name,
                "sphere",
                &sampled_sphere(*center, *radius, radial_segments),
            )?)
        }
        RagdollShapeIr::Capsule {
            vertex_a,
            vertex_b,
            radius,
        } => {
            let mut vertices = sampled_sphere(*vertex_a, *radius, radial_segments);
            vertices.extend(sampled_sphere(*vertex_b, *radius, radial_segments));
            RagdollShapeIr::ConvexHull(sampled_primitive_hull(body_name, "capsule", &vertices)?)
        }
    })
}

pub fn extract_skyrim_2010_creature_ragdoll(
    source: &HkxFile,
) -> Result<CreatureRagdollIr, CreatureRagdollError> {
    if source.class_version() != 8
        || source.contents_version() != "hk_2010.2.0-r1"
        || source.packfile().header.pointer_size != 8
    {
        return Err(CreatureRagdollError::UnsupportedSkyrimSource {
            class_version: source.class_version(),
            contents_version: source.contents_version().to_string(),
            pointer_size: source.packfile().header.pointer_size,
        });
    }
    let raw = SkyrimRawSource::new(source)?;
    let ragdoll_instance_index = unique_source_object_index(source, "hkaRagdollInstance")?;
    let ragdoll_instance = &source.objects()[ragdoll_instance_index];
    let ragdoll_skeleton_index = raw.object_pointer(ragdoll_instance.offset + 64)?;

    let mapper_candidates = source
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, object)| object.class_name == "hkaSkeletonMapper")
        .filter_map(|(index, object)| {
            let skeleton_a = raw.object_pointer(object.offset + 16).ok()?;
            let skeleton_b = raw.object_pointer(object.offset + 24).ok()?;
            (skeleton_a == ragdoll_skeleton_index).then_some((index, skeleton_b))
        })
        .collect::<Vec<_>>();
    if mapper_candidates.len() != 1 {
        return Err(CreatureRagdollError::SkyrimSourceMapperMissing);
    }
    let (mapper_index, animation_skeleton_index) = mapper_candidates[0];

    let animation_skeleton = parse_source_skeleton(&source.objects()[animation_skeleton_index])?;
    let ragdoll_skeleton = parse_source_skeleton(&source.objects()[ragdoll_skeleton_index])?;
    let mappings = parse_source_mapper(
        &raw,
        &source.objects()[mapper_index],
        &ragdoll_skeleton,
        &animation_skeleton,
    )?;

    let body_object_indices = raw.pointer_array(ragdoll_instance.offset + 16)?;
    let constraint_object_indices = raw.pointer_array(ragdoll_instance.offset + 32)?;
    let bone_to_body = raw.i32_array(ragdoll_instance.offset + 48)?;
    if bone_to_body.len() != ragdoll_skeleton.bones.len() {
        return Err(CreatureRagdollError::SkyrimSourceBoneBodyMap {
            reason: format!(
                "{} entries for {} ragdoll bones",
                bone_to_body.len(),
                ragdoll_skeleton.bones.len()
            ),
        });
    }
    let mut body_bones = vec![None; body_object_indices.len()];
    for (bone_index, body_index) in bone_to_body.iter().copied().enumerate() {
        if body_index < 0 {
            continue;
        }
        let body_index = body_index as usize;
        let Some(slot) = body_bones.get_mut(body_index) else {
            return Err(CreatureRagdollError::SkyrimSourceBoneBodyMap {
                reason: format!("bone {bone_index} references missing body {body_index}"),
            });
        };
        if slot.replace(bone_index).is_some() {
            return Err(CreatureRagdollError::SkyrimSourceBoneBodyMap {
                reason: format!("body {body_index} is linked from multiple bones"),
            });
        }
    }
    if let Some(body_index) = body_bones.iter().position(Option::is_none) {
        return Err(CreatureRagdollError::SkyrimSourceBoneBodyMap {
            reason: format!("ragdoll instance body {body_index} has no skeleton bone"),
        });
    }

    let mut bodies = Vec::with_capacity(body_object_indices.len());
    let mut body_name_by_object = HashMap::new();
    for (body_index, object_index) in body_object_indices.iter().copied().enumerate() {
        let bone_index = body_bones[body_index].expect("complete map checked above");
        let body = parse_skyrim_source_body(
            &raw,
            &source.objects()[object_index],
            &ragdoll_skeleton.bones[bone_index].name,
        )?;
        body_name_by_object.insert(object_index, body.name.clone());
        bodies.push(body);
    }

    let constraints = constraint_object_indices
        .iter()
        .copied()
        .enumerate()
        .map(|(index, object_index)| {
            parse_skyrim_source_constraint(
                &raw,
                &source.objects()[object_index],
                index,
                &body_name_by_object,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let ir = CreatureRagdollIr {
        name: format!("{} Owned Ragdoll", ragdoll_skeleton.name),
        animation_skeleton,
        ragdoll_skeleton,
        bodies,
        constraints,
        mappings,
    };
    validate_creature_ragdoll_ir(&ir)?;
    Ok(ir)
}

pub fn reconstruct_fo4_creature_ragdoll_packfile(
    ir: &CreatureRagdollIr,
) -> Result<Vec<u8>, CreatureRagdollError> {
    let file = reconstruct_fo4_creature_ragdoll(ir)?;
    let bytes = file.save();
    let reread = HkxFile::read(&bytes).map_err(|error| CreatureRagdollError::PackfileReread {
        reason: error.to_string(),
    })?;
    validate_fo4_creature_ragdoll_file(&reread, ir.bodies.len(), ir.constraints.len())?;
    Ok(bytes)
}

pub fn reconstruct_fo4_embedded_creature_ragdoll(
    ir: &CreatureRagdollIr,
) -> Result<EmbeddedCreatureRagdoll, CreatureRagdollError> {
    validate_creature_ragdoll_ir(ir)?;
    reject_unsupported_target_types(ir)?;

    let bodies = ir
        .bodies
        .iter()
        .map(|body| embedded_body_shape(&body.shape))
        .collect::<Result<Vec<_>, _>>()?;
    let body_metas = ir
        .bodies
        .iter()
        .map(|body| {
            let layer = (body.collision_filter_info & 0xff) as u8;
            BodyMeta {
                layer: layer.max(1),
                collision_filter_info: Some(body.collision_filter_info),
                body_flags: Some(128),
                position: [
                    body.world_from_body.translation[0],
                    body.world_from_body.translation[1],
                    body.world_from_body.translation[2],
                    0.0,
                ],
                orientation: body.world_from_body.rotation,
                body_mass: Some(body.mass_properties.mass),
                ..BodyMeta::default()
            }
        })
        .collect::<Vec<_>>();
    let constraints = embedded_constraints(ir)?;
    let options = BuildOptions {
        friction: ir.bodies[0].friction,
        restitution: ir.bodies[0].restitution,
        layer: body_metas[0].layer,
        mass: ir.bodies[0].mass_properties.mass,
        ..BuildOptions::default()
    };
    let physics_blob = build_fo4_multi_body_collision_with_constraints(
        &bodies,
        &options,
        None,
        Some(&body_metas),
        Some(&constraints),
    )
    .map_err(|error| CreatureRagdollError::EmbeddedBuild {
        reason: error.to_string(),
    })?;
    let physics =
        HkxFile::read(&physics_blob).map_err(|error| CreatureRagdollError::PackfileReread {
            reason: error.to_string(),
        })?;
    let mut objects = physics.objects().to_vec();
    let root = objects
        .first_mut()
        .filter(|object| object.class_name == "hknpPhysicsSystemData")
        .ok_or_else(|| CreatureRagdollError::EmbeddedBuild {
            reason: "rebuilt physics has no root hknpPhysicsSystemData".to_string(),
        })?;
    root.class_name = "hknpRagdollData".to_string();

    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    let skeleton_index = push_skeleton(&mut registry, &mut objects, &ir.ragdoll_skeleton)?;
    let ragdoll_bone_indices = bone_indices(&ir.ragdoll_skeleton);
    let mut bone_to_body = vec![HkxValue::I32(-1); ir.ragdoll_skeleton.bones.len()];
    for (body_index, body) in ir.bodies.iter().enumerate() {
        bone_to_body[ragdoll_bone_indices[body.ragdoll_bone.as_str()]] =
            HkxValue::I32(body_index as i32);
    }
    objects[0].members.extend([
        member("skeleton", HkxValue::Pointer(Some(skeleton_index))),
        member("boneToBodyMap", HkxValue::Array(bone_to_body)),
    ]);

    let embedded = HkxFile::from_tagxml(FO4_CLASS_VERSION, FO4_CONTENTS_VERSION, objects);
    let binary_data = embedded.save();
    let reread =
        HkxFile::read(&binary_data).map_err(|error| CreatureRagdollError::PackfileReread {
            reason: error.to_string(),
        })?;
    validate_fo4_embedded_creature_ragdoll_file(
        &reread,
        ir.bodies.len(),
        ir.constraints.len(),
        ir.ragdoll_skeleton.bones.len(),
    )?;

    let animation_bone_by_ragdoll = ir
        .mappings
        .iter()
        .map(|mapping| {
            (
                mapping.ragdoll_bone.as_str(),
                mapping.animation_bone.as_str(),
            )
        })
        .collect::<HashMap<_, _>>();
    let animation_bone_targets = ir
        .bodies
        .iter()
        .map(|body| animation_bone_by_ragdoll[body.ragdoll_bone.as_str()].to_string())
        .collect();
    Ok(EmbeddedCreatureRagdoll {
        binary_data,
        animation_bone_targets,
    })
}

pub fn validate_fo4_embedded_creature_ragdoll_file(
    file: &HkxFile,
    expected_bodies: usize,
    expected_constraints: usize,
    expected_bones: usize,
) -> Result<(), CreatureRagdollError> {
    if file.class_version() != FO4_CLASS_VERSION || file.contents_version() != FO4_CONTENTS_VERSION
    {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!(
                "expected classversion {FO4_CLASS_VERSION} and {FO4_CONTENTS_VERSION}, found {} and {}",
                file.class_version(),
                file.contents_version()
            ),
        });
    }
    let root = file
        .objects()
        .first()
        .filter(|object| object.class_name == "hknpRagdollData")
        .ok_or_else(|| CreatureRagdollError::RereadContract {
            reason: "embedded ragdoll root is not hknpRagdollData".to_string(),
        })?;
    require_array_len(root, "bodyCinfos", expected_bodies)?;
    require_array_len(root, "motionCinfos", expected_bodies)?;
    require_array_len(root, "constraintCinfos", expected_constraints)?;
    require_array_len(root, "boneToBodyMap", expected_bones)?;
    let skeleton_index = pointer_member(root, "skeleton")?;
    let skeleton =
        file.objects()
            .get(skeleton_index)
            .ok_or_else(|| CreatureRagdollError::RereadContract {
                reason: "embedded ragdoll skeleton pointer is out of range".to_string(),
            })?;
    if skeleton.class_name != "hkaSkeleton" {
        return Err(CreatureRagdollError::RereadContract {
            reason: "embedded ragdoll skeleton pointer does not target hkaSkeleton".to_string(),
        });
    }
    require_array_len(skeleton, "bones", expected_bones)?;
    require_array_len(skeleton, "parentIndices", expected_bones)?;
    require_array_len(skeleton, "referencePose", expected_bones)?;

    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    for (object_index, object) in file.objects().iter().enumerate() {
        validate_object_descriptor(&mut registry, object)?;
        for value in object.members.iter().map(|member| &member.value) {
            validate_value_pointers(object_index, value, file.objects().len())?;
            validate_value_finite(&format!("objects[{object_index}]"), value)?;
        }
    }
    Ok(())
}

fn embedded_body_shape(shape: &RagdollShapeIr) -> Result<MultiBodyShape, CreatureRagdollError> {
    match shape {
        RagdollShapeIr::ConvexHull(hull) => Ok(MultiBodyShape::Polytope {
            vertices: hull.vertices.clone(),
        }),
        RagdollShapeIr::Compound { .. } => {
            let mut children = Vec::new();
            collect_embedded_compound_children(shape, &mut children)?;
            Ok(MultiBodyShape::Compound { children })
        }
        unsupported => Err(CreatureRagdollError::UnsupportedTargetShape {
            body: "embedded_ragdoll".to_string(),
            shape: unsupported.kind_name().to_string(),
        }),
    }
}

fn collect_embedded_compound_children(
    shape: &RagdollShapeIr,
    children: &mut Vec<CompoundChild>,
) -> Result<(), CreatureRagdollError> {
    match shape {
        RagdollShapeIr::ConvexHull(hull) => children.push(CompoundChild {
            transform: CompoundChild::identity_transform(),
            kind: CompoundChildKind::Polytope {
                vertices: hull.vertices.clone(),
            },
        }),
        RagdollShapeIr::Compound { children: nested } => {
            for child in nested {
                collect_embedded_compound_children(child, children)?;
            }
        }
        unsupported => {
            return Err(CreatureRagdollError::UnsupportedTargetShape {
                body: "embedded_ragdoll".to_string(),
                shape: unsupported.kind_name().to_string(),
            });
        }
    }
    Ok(())
}

fn embedded_constraints(
    ir: &CreatureRagdollIr,
) -> Result<GraftedConstraints, CreatureRagdollError> {
    let body_indices = ir
        .bodies
        .iter()
        .enumerate()
        .map(|(index, body)| (body.name.as_str(), index as u32))
        .collect::<HashMap<_, _>>();
    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    let mut objects = Vec::new();
    let mut cinfos = Vec::with_capacity(ir.constraints.len());
    for constraint in &ir.constraints {
        let data_object = push_constraint_data(&mut registry, &mut objects, constraint)?;
        cinfos.push(GraftCinfo {
            body_a: body_indices[constraint.body_a.as_str()],
            body_b: body_indices[constraint.body_b.as_str()],
            data_object,
            flags: 0,
        });
    }
    Ok(GraftedConstraints { objects, cinfos })
}

pub fn validate_fo4_creature_ragdoll_file(
    file: &HkxFile,
    expected_bodies: usize,
    expected_constraints: usize,
) -> Result<(), CreatureRagdollError> {
    if file.class_version() != FO4_CLASS_VERSION || file.contents_version() != FO4_CONTENTS_VERSION
    {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!(
                "expected classversion {FO4_CLASS_VERSION} and {FO4_CONTENTS_VERSION}, found {} and {}",
                file.class_version(),
                file.contents_version()
            ),
        });
    }

    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    for (object_index, object) in file.objects().iter().enumerate() {
        validate_object_descriptor(&mut registry, object)?;
        for member in &object.members {
            validate_value_pointers(object_index, &member.value, file.objects().len())?;
            validate_value_finite(
                &format!("objects[{object_index}].{}", member.name),
                &member.value,
            )?;
        }
    }

    require_class_count(file, "hkRootLevelContainer", 1)?;
    require_class_count(file, "hkaSkeleton", 2)?;
    require_class_count(file, "hkaSkeletonMapper", 2)?;
    require_class_count(file, "hkaRagdollInstance", 1)?;
    require_class_count(file, "hkpPhysicsSystem", 1)?;
    require_class_count(file, "hkpPhysicsData", 1)?;
    require_class_count(file, "hkpRigidBody", expected_bodies)?;
    let body_shape_indices = file
        .objects()
        .iter()
        .filter(|object| object.class_name == "hkpRigidBody")
        .map(rigid_body_shape_index)
        .collect::<Result<HashSet<_>, _>>()?;
    if body_shape_indices.len() != expected_bodies
        || body_shape_indices.iter().any(|index| {
            !matches!(
                file.objects()[*index].class_name.as_str(),
                "hkpConvexVerticesShape" | "hkpListShape"
            )
        })
    {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!(
                "expected {expected_bodies} supported body shape roots, found {}",
                body_shape_indices.len()
            ),
        });
    }
    require_class_count(file, "hkpConstraintInstance", expected_constraints * 2)?;
    let constraint_data_indices = file
        .objects()
        .iter()
        .filter(|object| object.class_name == "hkpConstraintInstance")
        .map(|object| pointer_member(object, "data"))
        .collect::<Result<HashSet<_>, _>>()?;
    if constraint_data_indices.len() != expected_constraints
        || constraint_data_indices.iter().any(|index| {
            !matches!(
                file.objects()[*index].class_name.as_str(),
                "hkpLimitedHingeConstraintData"
                    | "hkpHingeConstraintData"
                    | "hkpPrismaticConstraintData"
                    | "hknpBreakableConstraintData"
                    | "hkpRagdollConstraintData"
                    | "hkpFixedConstraintData"
            )
        })
    {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!(
                "expected {expected_constraints} supported constraint-data roots, found {}",
                constraint_data_indices.len()
            ),
        });
    }

    let ragdoll = unique_object(file, "hkaRagdollInstance")?;
    require_array_len(ragdoll, "rigidBodies", expected_bodies)?;
    require_array_len(ragdoll, "constraints", expected_constraints)?;
    let ragdoll_skeleton_index = pointer_member(ragdoll, "skeleton")?;
    let ragdoll_skeleton = &file.objects()[ragdoll_skeleton_index];
    let ragdoll_bone_count = array_len(ragdoll_skeleton, "bones")?;
    require_array_len(ragdoll, "boneToRigidBodyMap", ragdoll_bone_count)?;

    let physics = unique_object(file, "hkpPhysicsSystem")?;
    require_array_len(physics, "rigidBodies", expected_bodies)?;
    require_array_len(physics, "constraints", expected_constraints)?;
    Ok(())
}

fn rigid_body_shape_index(body: &HkxObject) -> Result<usize, CreatureRagdollError> {
    let collidable = body
        .members
        .iter()
        .find(|member| member.name == "collidable")
        .and_then(|member| member.value.as_object_members())
        .ok_or_else(|| CreatureRagdollError::RereadContract {
            reason: format!(
                "{} has no collidable object",
                body.name.as_deref().unwrap_or("body")
            ),
        })?;
    collidable
        .iter()
        .find(|member| member.name == "shape")
        .and_then(|member| match member.value {
            HkxValue::Pointer(Some(index)) => Some(index),
            _ => None,
        })
        .ok_or_else(|| CreatureRagdollError::RereadContract {
            reason: format!(
                "{} has no shape pointer",
                body.name.as_deref().unwrap_or("body")
            ),
        })
}

fn validate_skeleton<'a>(
    label: &str,
    skeleton: &'a RigSkeletonIr,
    require_single_root: bool,
) -> Result<HashMap<&'a str, usize>, CreatureRagdollError> {
    if skeleton.bones.is_empty() {
        return Err(CreatureRagdollError::EmptySkeleton {
            skeleton: label.to_string(),
        });
    }
    let roots = skeleton
        .bones
        .iter()
        .filter(|bone| bone.parent.is_none())
        .count();
    if roots == 0 || (require_single_root && roots != 1) {
        return Err(CreatureRagdollError::InvalidRootCount {
            skeleton: label.to_string(),
            roots,
        });
    }
    let mut indices = HashMap::new();
    for (index, bone) in skeleton.bones.iter().enumerate() {
        if indices.insert(bone.name.as_str(), index).is_some() {
            return Err(CreatureRagdollError::DuplicateBone {
                skeleton: label.to_string(),
                bone: bone.name.clone(),
            });
        }
        if let Some(parent) = bone.parent {
            if parent >= index {
                return Err(CreatureRagdollError::InvalidParent {
                    skeleton: label.to_string(),
                    bone: bone.name.clone(),
                    parent,
                });
            }
        }
        validate_qs_transform(
            &format!("{label}.bones[{index}].reference_pose"),
            &bone.reference_pose,
        )?;
    }
    Ok(indices)
}

fn validate_body(body: &RigidBodyIr) -> Result<(), CreatureRagdollError> {
    validate_qs_transform(
        &format!("bodies[{}].world_from_body", body.name),
        &body.world_from_body,
    )?;
    validate_positive(
        &format!("bodies[{}].mass", body.name),
        body.mass_properties.mass,
    )?;
    for (axis, value) in body.mass_properties.inertia_diagonal.iter().enumerate() {
        validate_positive(&format!("bodies[{}].inertia[{axis}]", body.name), *value)?;
    }
    validate_finite_slice(
        &format!("bodies[{}].center_of_mass", body.name),
        &body.mass_properties.center_of_mass,
    )?;
    for (name, value) in [
        ("friction", body.friction),
        ("restitution", body.restitution),
        ("linear_damping", body.linear_damping),
        ("angular_damping", body.angular_damping),
    ] {
        if !value.is_finite() {
            return Err(CreatureRagdollError::NonFinite {
                path: format!("bodies[{}].{name}", body.name),
            });
        }
        if value < 0.0 {
            return Err(CreatureRagdollError::Negative {
                path: format!("bodies[{}].{name}", body.name),
            });
        }
    }
    validate_shape(body, &body.shape, &format!("bodies[{}].shape", body.name))?;
    Ok(())
}

fn validate_shape(
    body: &RigidBodyIr,
    shape: &RagdollShapeIr,
    path: &str,
) -> Result<(), CreatureRagdollError> {
    match shape {
        RagdollShapeIr::ConvexHull(hull) => validate_convex_hull(body, hull)?,
        RagdollShapeIr::Compound { children } => {
            if children.is_empty() {
                return Err(CreatureRagdollError::UnsupportedTargetShape {
                    body: body.name.clone(),
                    shape: "empty_compound".to_string(),
                });
            }
            for (index, child) in children.iter().enumerate() {
                validate_shape(body, child, &format!("{path}.children[{index}]"))?;
            }
        }
        RagdollShapeIr::Capsule {
            vertex_a,
            vertex_b,
            radius,
        } => {
            validate_finite_slice(&format!("bodies[{}].capsule.vertex_a", body.name), vertex_a)?;
            validate_finite_slice(&format!("bodies[{}].capsule.vertex_b", body.name), vertex_b)?;
            validate_positive(&format!("bodies[{}].capsule.radius", body.name), *radius)?;
        }
        RagdollShapeIr::Sphere { center, radius } => {
            validate_finite_slice(&format!("bodies[{}].sphere.center", body.name), center)?;
            validate_positive(&format!("bodies[{}].sphere.radius", body.name), *radius)?;
        }
        RagdollShapeIr::Box {
            half_extents,
            convex_radius,
        } => {
            for (axis, value) in half_extents.iter().enumerate() {
                validate_positive(
                    &format!("bodies[{}].box.half_extents[{axis}]", body.name),
                    *value,
                )?;
            }
            if !convex_radius.is_finite() {
                return Err(CreatureRagdollError::NonFinite {
                    path: format!("bodies[{}].box.convex_radius", body.name),
                });
            }
            if *convex_radius < 0.0 {
                return Err(CreatureRagdollError::Negative {
                    path: format!("bodies[{}].box.convex_radius", body.name),
                });
            }
        }
    }
    let _ = path;
    Ok(())
}

fn validate_convex_hull(
    body: &RigidBodyIr,
    hull: &ConvexHullIr,
) -> Result<(), CreatureRagdollError> {
    if hull.vertices.len() < 4 || hull.planes.len() < 4 {
        return Err(CreatureRagdollError::InvalidConvexHull {
            body: body.name.clone(),
        });
    }
    validate_finite_slice(
        &format!("bodies[{}].convex_radius", body.name),
        &[hull.convex_radius],
    )?;
    if hull.convex_radius < 0.0 {
        return Err(CreatureRagdollError::Negative {
            path: format!("bodies[{}].convex_radius", body.name),
        });
    }
    for (index, vertex) in hull.vertices.iter().enumerate() {
        validate_finite_slice(&format!("bodies[{}].vertices[{index}]", body.name), vertex)?;
    }
    for (index, plane) in hull.planes.iter().enumerate() {
        validate_finite_slice(&format!("bodies[{}].planes[{index}]", body.name), plane)?;
        let normal_length_squared = plane[..3].iter().map(|value| value * value).sum::<f32>();
        if normal_length_squared <= f32::EPSILON {
            return Err(CreatureRagdollError::InvalidPlane {
                body: body.name.clone(),
                plane: index,
            });
        }
    }
    Ok(())
}

fn validate_constraint(constraint: &ConstraintIr) -> Result<(), CreatureRagdollError> {
    validate_constraint_kind(&constraint.name, &constraint.kind)
}

fn validate_constraint_kind(
    constraint_name: &str,
    kind: &ConstraintKindIr,
) -> Result<(), CreatureRagdollError> {
    match kind {
        ConstraintKindIr::LimitedHinge(hinge) => {
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_a"),
                &hinge.frame_a,
            )?;
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_b"),
                &hinge.frame_b,
            )?;
            validate_finite_slice(
                &format!("constraints[{constraint_name}].angles"),
                &[hinge.min_angle, hinge.max_angle, hinge.max_friction_torque],
            )?;
            if hinge.min_angle > hinge.max_angle {
                return Err(CreatureRagdollError::InvalidAngleRange {
                    constraint: constraint_name.to_string(),
                });
            }
            if hinge.max_friction_torque < 0.0 {
                return Err(CreatureRagdollError::Negative {
                    path: format!("constraints[{constraint_name}].max_friction_torque"),
                });
            }
        }
        ConstraintKindIr::Hinge(hinge) => {
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_a"),
                &hinge.frame_a,
            )?;
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_b"),
                &hinge.frame_b,
            )?;
        }
        ConstraintKindIr::Prismatic(prismatic) => {
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_a"),
                &prismatic.frame_a,
            )?;
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_b"),
                &prismatic.frame_b,
            )?;
            validate_finite_slice(
                &format!("constraints[{constraint_name}].prismatic"),
                &[
                    prismatic.min_distance,
                    prismatic.max_distance,
                    prismatic.max_friction_force,
                ],
            )?;
            if prismatic.min_distance > prismatic.max_distance {
                return Err(CreatureRagdollError::InvalidAngleRange {
                    constraint: constraint_name.to_string(),
                });
            }
            if prismatic.max_friction_force < 0.0 {
                return Err(CreatureRagdollError::Negative {
                    path: format!("constraints[{constraint_name}].max_friction_force"),
                });
            }
        }
        ConstraintKindIr::Breakable { inner, threshold } => {
            validate_finite_slice(
                &format!("constraints[{constraint_name}].break_threshold"),
                &[*threshold],
            )?;
            if *threshold <= 0.0 {
                return Err(CreatureRagdollError::NonPositive {
                    path: format!("constraints[{constraint_name}].break_threshold"),
                });
            }
            if matches!(inner.as_ref(), ConstraintKindIr::Breakable { .. }) {
                return Err(CreatureRagdollError::UnsupportedTargetConstraint {
                    constraint: constraint_name.to_string(),
                    kind: "nested_breakable".to_string(),
                });
            }
            validate_constraint_kind(constraint_name, inner)?;
        }
        ConstraintKindIr::Ragdoll(limits) => {
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_a"),
                &limits.frame_a,
            )?;
            validate_qs_transform(
                &format!("constraints[{constraint_name}].frame_b"),
                &limits.frame_b,
            )?;
            validate_finite_slice(
                &format!("constraints[{constraint_name}].limits"),
                &[
                    limits.cone_limit,
                    limits.plane_min,
                    limits.plane_max,
                    limits.twist_min,
                    limits.twist_max,
                    limits.max_friction_torque,
                ],
            )?;
            if limits.plane_min > limits.plane_max || limits.twist_min > limits.twist_max {
                return Err(CreatureRagdollError::InvalidAngleRange {
                    constraint: constraint_name.to_string(),
                });
            }
            if limits.cone_limit < 0.0 || limits.max_friction_torque < 0.0 {
                return Err(CreatureRagdollError::Negative {
                    path: format!("constraints[{constraint_name}].ragdoll_limit"),
                });
            }
        }
        ConstraintKindIr::Fixed { frame_a, frame_b } => {
            validate_qs_transform(&format!("constraints[{constraint_name}].frame_a"), frame_a)?;
            validate_qs_transform(&format!("constraints[{constraint_name}].frame_b"), frame_b)?;
        }
    }
    Ok(())
}

fn reject_unsupported_target_types(ir: &CreatureRagdollIr) -> Result<(), CreatureRagdollError> {
    for body in &ir.bodies {
        if !target_shape_supported(&body.shape) {
            return Err(CreatureRagdollError::UnsupportedTargetShape {
                body: body.name.clone(),
                shape: body.shape.kind_name().to_string(),
            });
        }
    }
    Ok(())
}

fn target_shape_supported(shape: &RagdollShapeIr) -> bool {
    match shape {
        RagdollShapeIr::ConvexHull(_) => true,
        RagdollShapeIr::Compound { children } => {
            !children.is_empty() && children.iter().all(target_shape_supported)
        }
        RagdollShapeIr::Capsule { .. }
        | RagdollShapeIr::Sphere { .. }
        | RagdollShapeIr::Box { .. } => false,
    }
}

fn validate_qs_transform(
    path: &str,
    transform: &QsTransformIr,
) -> Result<(), CreatureRagdollError> {
    validate_finite_slice(path, &transform.translation)?;
    validate_finite_slice(path, &transform.rotation)?;
    validate_finite_slice(path, &transform.scale)?;
    let quaternion_length = transform
        .rotation
        .iter()
        .map(|value| value * value)
        .sum::<f32>();
    if quaternion_length <= f32::EPSILON {
        return Err(CreatureRagdollError::NonPositive {
            path: format!("{path}.rotation_length"),
        });
    }
    for (axis, scale) in transform.scale.iter().enumerate() {
        if scale.abs() <= f32::EPSILON {
            return Err(CreatureRagdollError::NonPositive {
                path: format!("{path}.scale[{axis}]"),
            });
        }
    }
    Ok(())
}

fn validate_finite_slice(path: &str, values: &[f32]) -> Result<(), CreatureRagdollError> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(CreatureRagdollError::NonFinite {
            path: path.to_string(),
        });
    }
    Ok(())
}

fn validate_positive(path: &str, value: f32) -> Result<(), CreatureRagdollError> {
    validate_finite_slice(path, &[value])?;
    if value <= 0.0 {
        return Err(CreatureRagdollError::NonPositive {
            path: path.to_string(),
        });
    }
    Ok(())
}

fn push_skeleton(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    skeleton: &RigSkeletonIr,
) -> Result<usize, CreatureRagdollError> {
    let parent_indices = skeleton
        .bones
        .iter()
        .map(|bone| HkxValue::I16(bone.parent.map(|parent| parent as i16).unwrap_or(-1)))
        .collect();
    let bones = skeleton
        .bones
        .iter()
        .map(|bone| {
            HkxValue::Object(vec![
                string_member("name", &bone.name),
                member("lockTranslation", HkxValue::Bool(bone.lock_translation)),
            ])
        })
        .collect();
    let reference_pose = skeleton
        .bones
        .iter()
        .map(|bone| qs_value(&bone.reference_pose))
        .collect();
    push_target_object(
        registry,
        objects,
        "hkaSkeleton",
        vec![
            string_member("name", &skeleton.name),
            member("parentIndices", HkxValue::Array(parent_indices)),
            member("bones", HkxValue::Array(bones)),
            member("referencePose", HkxValue::Array(reference_pose)),
            member("referenceFloats", HkxValue::Array(Vec::new())),
            member("floatSlots", HkxValue::Array(Vec::new())),
            member("localFrames", HkxValue::Array(Vec::new())),
            member("partitions", HkxValue::Array(Vec::new())),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn push_mapper(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    skeleton_a: usize,
    skeleton_b: usize,
    mappings: &[BoneMappingIr],
    ragdoll_bones: &HashMap<&str, usize>,
    animation_bones: &HashMap<&str, usize>,
    reverse: bool,
) -> Result<usize, CreatureRagdollError> {
    let simple_mappings = mappings
        .iter()
        .map(|mapping| {
            let (bone_a, bone_b, transform) = if reverse {
                (
                    animation_bones[mapping.animation_bone.as_str()],
                    ragdoll_bones[mapping.ragdoll_bone.as_str()],
                    inverse_qs_transform(&mapping.ragdoll_from_animation),
                )
            } else {
                (
                    ragdoll_bones[mapping.ragdoll_bone.as_str()],
                    animation_bones[mapping.animation_bone.as_str()],
                    mapping.ragdoll_from_animation.clone(),
                )
            };
            HkxValue::Object(vec![
                member("boneA", HkxValue::I16(bone_a as i16)),
                member("boneB", HkxValue::I16(bone_b as i16)),
                member("aFromBTransform", qs_value(&transform)),
            ])
        })
        .collect();
    let mapped_a: HashSet<usize> = mappings
        .iter()
        .map(|mapping| {
            if reverse {
                animation_bones[mapping.animation_bone.as_str()]
            } else {
                ragdoll_bones[mapping.ragdoll_bone.as_str()]
            }
        })
        .collect();
    let a_bone_count = if reverse {
        animation_bones.len()
    } else {
        ragdoll_bones.len()
    };
    let unmapped_bones = (0..a_bone_count)
        .filter(|index| !mapped_a.contains(index))
        .map(|index| HkxValue::I16(index as i16))
        .collect();
    let mapping = HkxValue::Object(vec![
        member("skeletonA", HkxValue::Pointer(Some(skeleton_a))),
        member("skeletonB", HkxValue::Pointer(Some(skeleton_b))),
        member("partitionMap", HkxValue::Array(Vec::new())),
        member("simpleMappingPartitionRanges", HkxValue::Array(Vec::new())),
        member("chainMappingPartitionRanges", HkxValue::Array(Vec::new())),
        member("simpleMappings", HkxValue::Array(simple_mappings)),
        member("chainMappings", HkxValue::Array(Vec::new())),
        member("unmappedBones", HkxValue::Array(unmapped_bones)),
        member(
            "extractedMotionMapping",
            qs_value(&QsTransformIr::identity()),
        ),
        member("keepUnmappedLocal", HkxValue::Bool(false)),
        member("mappingType", HkxValue::I32(0)),
    ]);
    push_target_object(
        registry,
        objects,
        "hkaSkeletonMapper",
        vec![member("mapping", mapping)],
    )
}

fn push_ragdoll_shape(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    shape: &RagdollShapeIr,
) -> Result<usize, CreatureRagdollError> {
    match shape {
        RagdollShapeIr::ConvexHull(hull) => push_convex_shape(registry, objects, hull),
        RagdollShapeIr::Compound { children } => {
            let child_indices = children
                .iter()
                .map(|child| push_ragdoll_shape(registry, objects, child))
                .collect::<Result<Vec<_>, _>>()?;
            let mut vertices = Vec::new();
            collect_shape_vertices(shape, &mut vertices);
            let (aabb_center, aabb_half_extents) = aabb(&vertices);
            let child_info = child_indices
                .into_iter()
                .map(|index| {
                    HkxValue::Object(vec![
                        member("shape", HkxValue::Pointer(Some(index))),
                        member("collisionFilterInfo", HkxValue::U32(0)),
                        member("shapeInfo", HkxValue::U16(0)),
                    ])
                })
                .collect();
            push_target_object(
                registry,
                objects,
                "hkpListShape",
                vec![
                    member("userData", HkxValue::U64(0)),
                    member("disableWelding", HkxValue::Bool(false)),
                    member("collectionType", HkxValue::U8(0)),
                    member("childInfo", HkxValue::Array(child_info)),
                    member("flags", HkxValue::U16(0)),
                    member("numDisabledChildren", HkxValue::U16(0)),
                    member(
                        "aabbHalfExtents",
                        vector4(aabb_half_extents, shape_convex_radius(shape)),
                    ),
                    member("aabbCenter", vector4(aabb_center, 1.0)),
                    member(
                        "enabledChildren",
                        HkxValue::Array(vec![HkxValue::U32(u32::MAX); 8]),
                    ),
                ],
            )
        }
        _ => unreachable!("unsupported target shapes are rejected before reconstruction"),
    }
}

fn push_convex_shape(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    hull: &ConvexHullIr,
) -> Result<usize, CreatureRagdollError> {
    let (aabb_center, aabb_half_extents) = aabb(&hull.vertices);
    let rotated_vertices = hull
        .vertices
        .chunks(4)
        .map(|chunk| {
            let mut points = [[0.0; 3]; 4];
            for index in 0..4 {
                points[index] = chunk
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| chunk[chunk.len() - 1]);
            }
            HkxValue::F32List(vec![
                points[0][0],
                points[1][0],
                points[2][0],
                points[3][0],
                points[0][1],
                points[1][1],
                points[2][1],
                points[3][1],
                points[0][2],
                points[1][2],
                points[2][2],
                points[3][2],
            ])
        })
        .collect();
    let plane_equations = hull
        .planes
        .iter()
        .map(|plane| HkxValue::F32List(plane.to_vec()))
        .collect();
    push_target_object(
        registry,
        objects,
        "hkpConvexVerticesShape",
        vec![
            member("userData", HkxValue::U64(0)),
            member("radius", HkxValue::F32(hull.convex_radius)),
            member(
                "aabbHalfExtents",
                HkxValue::F32List(vec![
                    aabb_half_extents[0],
                    aabb_half_extents[1],
                    aabb_half_extents[2],
                    0.0,
                ]),
            ),
            member(
                "aabbCenter",
                HkxValue::F32List(vec![aabb_center[0], aabb_center[1], aabb_center[2], 0.0]),
            ),
            member("rotatedVertices", HkxValue::Array(rotated_vertices)),
            member("numVertices", HkxValue::I32(hull.vertices.len() as i32)),
            member("planeEquations", HkxValue::Array(plane_equations)),
            member("connectivity", HkxValue::Pointer(None)),
        ],
    )
}

fn collect_shape_vertices(shape: &RagdollShapeIr, vertices: &mut Vec<[f32; 3]>) {
    match shape {
        RagdollShapeIr::ConvexHull(hull) => vertices.extend_from_slice(&hull.vertices),
        RagdollShapeIr::Compound { children } => {
            for child in children {
                collect_shape_vertices(child, vertices);
            }
        }
        _ => unreachable!("unsupported target shapes are rejected before reconstruction"),
    }
}

fn shape_convex_radius(shape: &RagdollShapeIr) -> f32 {
    match shape {
        RagdollShapeIr::ConvexHull(hull) => hull.convex_radius,
        RagdollShapeIr::Compound { children } => {
            children.iter().map(shape_convex_radius).fold(0.0, f32::max)
        }
        _ => unreachable!("unsupported target shapes are rejected before reconstruction"),
    }
}

fn push_rigid_body(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    body: &RigidBodyIr,
    shape_index: usize,
) -> Result<usize, CreatureRagdollError> {
    let object_radius = body_shape_radius(body);
    let world_center_of_mass =
        transform_point(&body.world_from_body, body.mass_properties.center_of_mass);
    let swept_transform = HkxValue::Array(vec![
        vector4(world_center_of_mass, 0.0),
        vector4(world_center_of_mass, 0.0),
        HkxValue::F32List(body.world_from_body.rotation.to_vec()),
        HkxValue::F32List(body.world_from_body.rotation.to_vec()),
        vector4(body.mass_properties.center_of_mass, 0.0),
    ]);
    let motion_state = HkxValue::Object(vec![
        member("transform", matrix_value(&body.world_from_body)),
        member("sweptTransform", swept_transform),
        member("deltaAngle", HkxValue::F32List(vec![0.0; 4])),
        member("objectRadius", HkxValue::F32(object_radius)),
        member("linearDamping", HkxValue::Half(body.linear_damping)),
        member("angularDamping", HkxValue::Half(body.angular_damping)),
        member("timeFactor", HkxValue::Half(1.0)),
        member(
            "maxLinearVelocity",
            HkxValue::Object(vec![member("value", HkxValue::U8(127))]),
        ),
        member(
            "maxAngularVelocity",
            HkxValue::Object(vec![member("value", HkxValue::U8(127))]),
        ),
        member("deactivationClass", HkxValue::U8(2)),
    ]);
    let motion = HkxValue::Object(vec![
        member("type", HkxValue::U8(3)),
        member("deactivationIntegrateCounter", HkxValue::U8(0)),
        member(
            "deactivationNumInactiveFrames",
            HkxValue::Array(vec![HkxValue::U16(0), HkxValue::U16(0)]),
        ),
        member("motionState", motion_state),
        member(
            "inertiaAndMassInv",
            HkxValue::F32List(vec![
                body.mass_properties.inertia_diagonal[0].recip(),
                body.mass_properties.inertia_diagonal[1].recip(),
                body.mass_properties.inertia_diagonal[2].recip(),
                body.mass_properties.mass.recip(),
            ]),
        ),
        member("linearVelocity", HkxValue::F32List(vec![0.0; 4])),
        member("angularVelocity", HkxValue::F32List(vec![0.0; 4])),
        member(
            "deactivationRefPosition",
            HkxValue::Array(vec![
                vector4(world_center_of_mass, 0.0),
                vector4(world_center_of_mass, 0.0),
            ]),
        ),
        member(
            "deactivationRefOrientation",
            HkxValue::Array(vec![HkxValue::U32(0), HkxValue::U32(0)]),
        ),
        member("savedMotion", HkxValue::Pointer(None)),
        member("savedQualityTypeIndex", HkxValue::U16(0)),
        member("gravityFactor", HkxValue::Half(1.0)),
    ]);
    let collidable = HkxValue::Object(vec![
        member("shape", HkxValue::Pointer(Some(shape_index))),
        member("shapeKey", HkxValue::U32(u32::MAX)),
        member("forceCollideOntoPpu", HkxValue::U8(8)),
        member(
            "broadPhaseHandle",
            HkxValue::Object(vec![
                member("type", HkxValue::I8(1)),
                member("objectQualityType", HkxValue::I8(4)),
                member(
                    "collisionFilterInfo",
                    HkxValue::U32(body.collision_filter_info),
                ),
            ]),
        ),
        member("allowedPenetrationDepth", HkxValue::F32(0.1)),
    ]);
    push_target_object(
        registry,
        objects,
        "hkpRigidBody",
        vec![
            member("userData", HkxValue::U64(0)),
            member("collidable", collidable),
            member("multiThreadCheck", HkxValue::Object(Vec::new())),
            string_member("name", &body.name),
            member("properties", HkxValue::Array(Vec::new())),
            member(
                "material",
                HkxValue::Object(vec![
                    member("responseType", HkxValue::I8(1)),
                    member("rollingFrictionMultiplier", HkxValue::Half(0.0)),
                    member("friction", HkxValue::F32(body.friction)),
                    member("restitution", HkxValue::F32(body.restitution)),
                ]),
            ),
            member("damageMultiplier", HkxValue::F32(1.0)),
            member("storageIndex", HkxValue::U16(0)),
            member("contactPointCallbackDelay", HkxValue::U16(0)),
            member("autoRemoveLevel", HkxValue::I8(0)),
            member("numShapeKeysInContactPointProperties", HkxValue::U8(0)),
            member("responseModifierFlags", HkxValue::U8(0)),
            member("uid", HkxValue::U32(u32::MAX)),
            member(
                "spuCollisionCallback",
                HkxValue::Object(vec![
                    member("eventFilter", HkxValue::U8(0)),
                    member("userFilter", HkxValue::U8(0)),
                ]),
            ),
            member("motion", motion),
            member("localFrame", HkxValue::Pointer(None)),
            member("npData", HkxValue::U32(0)),
        ],
    )
}

fn push_limited_hinge_constraint_data(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    hinge: &LimitedHingeIr,
) -> Result<usize, CreatureRagdollError> {
    let atoms = HkxValue::Object(vec![
        member(
            "transforms",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(2)),
                member("transformA", matrix_value(&hinge.frame_a)),
                member("transformB", matrix_value(&hinge.frame_b)),
            ]),
        ),
        member(
            "setupStabilization",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(23)),
                member("enabled", HkxValue::Bool(true)),
                member("maxLinImpulse", HkxValue::F32(f32::MAX)),
                member("maxAngImpulse", HkxValue::F32(f32::MAX)),
                member("maxAngle", HkxValue::F32(f32::MAX)),
            ]),
        ),
        member(
            "angMotor",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(18)),
                member("isEnabled", HkxValue::Bool(false)),
                member("motorAxis", HkxValue::U8(0)),
                member("targetAngle", HkxValue::F32(0.0)),
                member("motor", HkxValue::Pointer(None)),
            ]),
        ),
        member(
            "angFriction",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(17)),
                member(
                    "isEnabled",
                    HkxValue::U8(u8::from(hinge.max_friction_torque > 0.0)),
                ),
                member("firstFrictionAxis", HkxValue::U8(0)),
                member("numFrictionAxes", HkxValue::U8(1)),
                member(
                    "maxFrictionTorque",
                    HkxValue::F32(hinge.max_friction_torque),
                ),
            ]),
        ),
        member(
            "angLimit",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(14)),
                member("isEnabled", HkxValue::U8(1)),
                member("limitAxis", HkxValue::U8(0)),
                member("minAngle", HkxValue::F32(hinge.min_angle)),
                member("maxAngle", HkxValue::F32(hinge.max_angle)),
                member("angularLimitsTauFactor", HkxValue::F32(1.0)),
            ]),
        ),
        member(
            "2dAng",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(12)),
                member("freeRotationAxis", HkxValue::U8(0)),
            ]),
        ),
        member(
            "ballSocket",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(5)),
                member("solvingMethod", HkxValue::U8(0)),
                member("bodiesToNotify", HkxValue::U8(0)),
                member(
                    "velocityStabilizationFactor",
                    HkxValue::Object(vec![member("value", HkxValue::U8(255))]),
                ),
                member("enableLinearImpulseLimit", HkxValue::Bool(false)),
                member("breachImpulse", HkxValue::F32(f32::MAX)),
                member("inertiaStabilizationFactor", HkxValue::F32(0.0)),
            ]),
        ),
    ]);
    push_target_object(
        registry,
        objects,
        "hkpLimitedHingeConstraintData",
        vec![member("userData", HkxValue::U64(0)), member("atoms", atoms)],
    )
}

fn push_constraint_data(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    constraint: &ConstraintIr,
) -> Result<usize, CreatureRagdollError> {
    push_constraint_kind(registry, objects, &constraint.kind)
}

fn push_constraint_kind(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    kind: &ConstraintKindIr,
) -> Result<usize, CreatureRagdollError> {
    match kind {
        ConstraintKindIr::LimitedHinge(hinge) => {
            push_limited_hinge_constraint_data(registry, objects, hinge)
        }
        ConstraintKindIr::Hinge(hinge) => push_hinge_constraint_data(registry, objects, hinge),
        ConstraintKindIr::Prismatic(prismatic) => {
            push_prismatic_constraint_data(registry, objects, prismatic)
        }
        ConstraintKindIr::Breakable { inner, threshold } => {
            let inner_index = push_constraint_kind(registry, objects, inner.as_ref())?;
            push_target_object(
                registry,
                objects,
                "hknpBreakableConstraintData",
                vec![
                    member("userData", HkxValue::U64(0)),
                    member("constraintData", HkxValue::Pointer(Some(inner_index))),
                    member("threshold", HkxValue::F32(*threshold)),
                ],
            )
        }
        ConstraintKindIr::Ragdoll(limits) => {
            push_ragdoll_constraint_data(registry, objects, limits)
        }
        ConstraintKindIr::Fixed { frame_a, frame_b } => {
            push_fixed_constraint_data(registry, objects, frame_a, frame_b)
        }
    }
}

fn push_hinge_constraint_data(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    hinge: &HingeIr,
) -> Result<usize, CreatureRagdollError> {
    let atoms = HkxValue::Object(vec![
        local_transforms_atom(&hinge.frame_a, &hinge.frame_b),
        setup_stabilization_atom(),
        member(
            "2dAng",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(12)),
                member("freeRotationAxis", HkxValue::U8(0)),
            ]),
        ),
        stabilized_ball_socket_atom(),
    ]);
    push_target_object(
        registry,
        objects,
        "hkpHingeConstraintData",
        vec![member("userData", HkxValue::U64(0)), member("atoms", atoms)],
    )
}

fn push_prismatic_constraint_data(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    prismatic: &PrismaticIr,
) -> Result<usize, CreatureRagdollError> {
    let atoms = HkxValue::Object(vec![
        local_transforms_atom(&prismatic.frame_a, &prismatic.frame_b),
        member(
            "motor",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(11)),
                member("isEnabled", HkxValue::Bool(false)),
                member("motorAxis", HkxValue::U8(0)),
                member("targetPosition", HkxValue::F32(0.0)),
                member("motor", HkxValue::Pointer(None)),
            ]),
        ),
        member(
            "friction",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(10)),
                member(
                    "isEnabled",
                    HkxValue::U8(u8::from(prismatic.max_friction_force > 0.0)),
                ),
                member("frictionAxis", HkxValue::U8(0)),
                member(
                    "maxFrictionForce",
                    HkxValue::F32(prismatic.max_friction_force),
                ),
            ]),
        ),
        member(
            "ang",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(13)),
                member("firstConstrainedAxis", HkxValue::U8(0)),
                member("numConstrainedAxes", HkxValue::U8(3)),
            ]),
        ),
        member(
            "lin0",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(7)),
                member("axisIndex", HkxValue::U8(1)),
            ]),
        ),
        member(
            "lin1",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(7)),
                member("axisIndex", HkxValue::U8(2)),
            ]),
        ),
        member(
            "linLimit",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(9)),
                member("axisIndex", HkxValue::U8(0)),
                member("min", HkxValue::F32(prismatic.min_distance)),
                member("max", HkxValue::F32(prismatic.max_distance)),
            ]),
        ),
    ]);
    push_target_object(
        registry,
        objects,
        "hkpPrismaticConstraintData",
        vec![member("userData", HkxValue::U64(0)), member("atoms", atoms)],
    )
}

fn push_ragdoll_constraint_data(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    limits: &RagdollLimitsIr,
) -> Result<usize, CreatureRagdollError> {
    let atoms = HkxValue::Object(vec![
        local_transforms_atom(&limits.frame_a, &limits.frame_b),
        setup_stabilization_atom(),
        member(
            "ragdollMotors",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(19)),
                member("isEnabled", HkxValue::Bool(false)),
                member(
                    "target_bRca",
                    HkxValue::F32List(vec![
                        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
                    ]),
                ),
                member(
                    "motors",
                    HkxValue::Array(vec![
                        HkxValue::Pointer(None),
                        HkxValue::Pointer(None),
                        HkxValue::Pointer(None),
                    ]),
                ),
            ]),
        ),
        member(
            "angFriction",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(17)),
                member(
                    "isEnabled",
                    HkxValue::U8(u8::from(limits.max_friction_torque > 0.0)),
                ),
                member("firstFrictionAxis", HkxValue::U8(0)),
                member("numFrictionAxes", HkxValue::U8(3)),
                member(
                    "maxFrictionTorque",
                    HkxValue::F32(limits.max_friction_torque),
                ),
            ]),
        ),
        member(
            "twistLimit",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(15)),
                member("isEnabled", HkxValue::U8(1)),
                member("twistAxis", HkxValue::U8(0)),
                member("refAxis", HkxValue::U8(1)),
                member("minAngle", HkxValue::F32(limits.twist_min)),
                member("maxAngle", HkxValue::F32(limits.twist_max)),
                member("angularLimitsTauFactor", HkxValue::F32(0.8)),
            ]),
        ),
        member(
            "coneLimit",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(16)),
                member("isEnabled", HkxValue::U8(1)),
                member("twistAxisInA", HkxValue::U8(0)),
                member("refAxisInB", HkxValue::U8(0)),
                member("angleMeasurementMode", HkxValue::U8(0)),
                member("memOffsetToAngleOffset", HkxValue::U8(56)),
                member("minAngle", HkxValue::F32(-100.0)),
                member("maxAngle", HkxValue::F32(limits.cone_limit)),
                member("angularLimitsTauFactor", HkxValue::F32(0.8)),
            ]),
        ),
        member(
            "planesLimit",
            HkxValue::Object(vec![
                member("type", HkxValue::U16(16)),
                member("isEnabled", HkxValue::U8(1)),
                member("twistAxisInA", HkxValue::U8(0)),
                member("refAxisInB", HkxValue::U8(1)),
                member("angleMeasurementMode", HkxValue::U8(1)),
                member("memOffsetToAngleOffset", HkxValue::U8(0)),
                member("minAngle", HkxValue::F32(limits.plane_min)),
                member("maxAngle", HkxValue::F32(limits.plane_max)),
                member("angularLimitsTauFactor", HkxValue::F32(0.8)),
            ]),
        ),
        stabilized_ball_socket_atom(),
    ]);
    push_target_object(
        registry,
        objects,
        "hkpRagdollConstraintData",
        vec![member("userData", HkxValue::U64(0)), member("atoms", atoms)],
    )
}

fn push_fixed_constraint_data(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    frame_a: &QsTransformIr,
    frame_b: &QsTransformIr,
) -> Result<usize, CreatureRagdollError> {
    let atoms = HkxValue::Object(vec![
        local_transforms_atom(frame_a, frame_b),
        setup_stabilization_atom(),
        stabilized_ball_socket_atom(),
        member(
            "ang",
            HkxValue::Object(vec![member("type", HkxValue::U16(13))]),
        ),
    ]);
    push_target_object(
        registry,
        objects,
        "hkpFixedConstraintData",
        vec![member("userData", HkxValue::U64(0)), member("atoms", atoms)],
    )
}

fn local_transforms_atom(frame_a: &QsTransformIr, frame_b: &QsTransformIr) -> HkxMember {
    member(
        "transforms",
        HkxValue::Object(vec![
            member("type", HkxValue::U16(2)),
            member("transformA", matrix_value(frame_a)),
            member("transformB", matrix_value(frame_b)),
        ]),
    )
}

fn setup_stabilization_atom() -> HkxMember {
    member(
        "setupStabilization",
        HkxValue::Object(vec![
            member("type", HkxValue::U16(23)),
            member("enabled", HkxValue::Bool(true)),
            member("maxLinImpulse", HkxValue::F32(f32::MAX)),
            member("maxAngImpulse", HkxValue::F32(f32::MAX)),
            member("maxAngle", HkxValue::F32(1.844_672_6e19)),
        ]),
    )
}

fn stabilized_ball_socket_atom() -> HkxMember {
    member(
        "ballSocket",
        HkxValue::Object(vec![
            member("type", HkxValue::U16(5)),
            member("solvingMethod", HkxValue::U8(1)),
            member("bodiesToNotify", HkxValue::U8(0)),
            member(
                "velocityStabilizationFactor",
                HkxValue::Object(vec![member("value", HkxValue::U8(48))]),
            ),
            member("enableLinearImpulseLimit", HkxValue::Bool(false)),
            member("breachImpulse", HkxValue::F32(f32::MAX)),
            member("inertiaStabilizationFactor", HkxValue::F32(0.0)),
        ]),
    )
}

fn push_constraint_instance(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    constraint: &ConstraintIr,
    data_index: usize,
    body_a_index: usize,
    body_b_index: usize,
) -> Result<usize, CreatureRagdollError> {
    push_target_object(
        registry,
        objects,
        "hkpConstraintInstance",
        vec![
            member("data", HkxValue::Pointer(Some(data_index))),
            member("constraintModifiers", HkxValue::Pointer(None)),
            member(
                "entities",
                HkxValue::Array(vec![
                    HkxValue::Pointer(Some(body_a_index)),
                    HkxValue::Pointer(Some(body_b_index)),
                ]),
            ),
            member("priority", HkxValue::U8(1)),
            member("wantRuntime", HkxValue::Bool(true)),
            member("destructionRemapInfo", HkxValue::U8(0)),
            string_member("name", &constraint.name),
            member("userData", HkxValue::U64(0)),
        ],
    )
}

fn push_target_object(
    registry: &mut DescriptorRegistry,
    objects: &mut Vec<HkxObject>,
    class_name: &str,
    members: Vec<HkxMember>,
) -> Result<usize, CreatureRagdollError> {
    let signature = descriptor_signature(registry, class_name)?;
    let templates = target_members(registry, class_name)?;
    for member in &members {
        if !templates
            .iter()
            .any(|template| template.name == member.name)
        {
            return Err(CreatureRagdollError::DescriptorMemberMissing {
                class_name: class_name.to_string(),
                member: member.name.clone(),
            });
        }
    }
    let index = objects.len();
    objects.push(HkxObject {
        name: Some(format!("#{:04}", index + 1)),
        offset: 0,
        signature,
        class_name: class_name.to_string(),
        members,
    });
    Ok(index)
}

fn validate_object_descriptor(
    registry: &mut DescriptorRegistry,
    object: &HkxObject,
) -> Result<(), CreatureRagdollError> {
    let signature = descriptor_signature(registry, &object.class_name)?;
    if object.signature != signature {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!(
                "{} signature {:#010x} != descriptor {signature:#010x}",
                object.class_name, object.signature
            ),
        });
    }
    let templates = target_members(registry, &object.class_name)?;
    for member in &object.members {
        let template = templates
            .iter()
            .find(|template| template.name == member.name)
            .ok_or_else(|| CreatureRagdollError::DescriptorMemberMissing {
                class_name: object.class_name.clone(),
                member: member.name.clone(),
            })?;
        validate_inline_descriptor(registry, &object.class_name, template, &member.value)?;
    }
    Ok(())
}

fn validate_inline_descriptor(
    registry: &mut DescriptorRegistry,
    owner_class: &str,
    template: &MemberTemplate,
    value: &HkxValue,
) -> Result<(), CreatureRagdollError> {
    if template.ctype.is_empty() {
        return Ok(());
    }
    let inline_values: Vec<&HkxValue> = match value {
        HkxValue::Object(_) | HkxValue::TypedObject { .. } => vec![value],
        HkxValue::Array(values) if template.vsubtype == HkxType::Struct => values.iter().collect(),
        _ => Vec::new(),
    };
    for inline in inline_values {
        let class_name = match inline {
            HkxValue::TypedObject { class_name, .. } => class_name.as_str(),
            _ => template.ctype.as_str(),
        };
        let templates = target_members(registry, class_name)?;
        if let Some(members) = inline.as_object_members() {
            for member in members {
                let child_template = templates
                    .iter()
                    .find(|candidate| candidate.name == member.name)
                    .ok_or_else(|| CreatureRagdollError::DescriptorMemberMissing {
                        class_name: class_name.to_string(),
                        member: member.name.clone(),
                    })?;
                validate_inline_descriptor(registry, class_name, child_template, &member.value)?;
            }
        }
    }
    let _ = owner_class;
    Ok(())
}

fn descriptor_signature(
    registry: &mut DescriptorRegistry,
    class_name: &str,
) -> Result<u32, CreatureRagdollError> {
    let descriptor = registry
        .get(class_name)
        .map_err(|error| CreatureRagdollError::Descriptor {
            class_name: class_name.to_string(),
            reason: error.to_string(),
        })?
        .cloned()
        .ok_or_else(|| CreatureRagdollError::DescriptorMissing {
            class_name: class_name.to_string(),
        })?;
    let signature = descriptor.signature.trim();
    let digits = signature
        .strip_prefix("0x")
        .or_else(|| signature.strip_prefix("0X"))
        .unwrap_or(signature);
    u32::from_str_radix(digits, 16).map_err(|_| CreatureRagdollError::InvalidSignature {
        class_name: class_name.to_string(),
        signature: descriptor.signature,
    })
}

fn target_members(
    registry: &mut DescriptorRegistry,
    class_name: &str,
) -> Result<Vec<MemberTemplate>, CreatureRagdollError> {
    let members =
        registry
            .get_all_members(class_name)
            .map_err(|error| CreatureRagdollError::Descriptor {
                class_name: class_name.to_string(),
                reason: error.to_string(),
            })?;
    if members.is_empty() && class_name != "hkReferencedObject" {
        let exists = registry
            .get(class_name)
            .map_err(|error| CreatureRagdollError::Descriptor {
                class_name: class_name.to_string(),
                reason: error.to_string(),
            })?
            .is_some();
        if !exists {
            return Err(CreatureRagdollError::DescriptorMissing {
                class_name: class_name.to_string(),
            });
        }
    }
    Ok(members)
}

fn validate_value_pointers(
    object: usize,
    value: &HkxValue,
    object_count: usize,
) -> Result<(), CreatureRagdollError> {
    match value {
        HkxValue::Pointer(Some(target)) if *target >= object_count => {
            return Err(CreatureRagdollError::PointerOutOfRange {
                object,
                target: *target,
            });
        }
        HkxValue::PendingPtr(target) => {
            return Err(CreatureRagdollError::PendingPointer {
                object,
                target: target.clone(),
            });
        }
        HkxValue::Array(values) => {
            for value in values {
                validate_value_pointers(object, value, object_count)?;
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for member in members {
                validate_value_pointers(object, &member.value, object_count)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_value_finite(path: &str, value: &HkxValue) -> Result<(), CreatureRagdollError> {
    match value {
        HkxValue::F32(value) | HkxValue::Half(value) if !value.is_finite() => {
            return Err(CreatureRagdollError::NonFinite {
                path: path.to_string(),
            });
        }
        HkxValue::F32List(values) if values.iter().any(|value| !value.is_finite()) => {
            return Err(CreatureRagdollError::NonFinite {
                path: path.to_string(),
            });
        }
        HkxValue::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_value_finite(&format!("{path}[{index}]"), value)?;
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for member in members {
                validate_value_finite(&format!("{path}.{}", member.name), &member.value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_class_count(
    file: &HkxFile,
    class_name: &str,
    expected: usize,
) -> Result<(), CreatureRagdollError> {
    let actual = file
        .objects()
        .iter()
        .filter(|object| object.class_name == class_name)
        .count();
    if actual != expected {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!("expected {expected} {class_name} objects, found {actual}"),
        });
    }
    Ok(())
}

fn class_count(file: &HkxFile, class_name: &str) -> usize {
    file.objects()
        .iter()
        .filter(|object| object.class_name == class_name)
        .count()
}

fn unique_object<'a>(
    file: &'a HkxFile,
    class_name: &str,
) -> Result<&'a HkxObject, CreatureRagdollError> {
    file.objects()
        .iter()
        .find(|object| object.class_name == class_name)
        .ok_or_else(|| CreatureRagdollError::RereadContract {
            reason: format!("missing {class_name}"),
        })
}

fn require_array_len(
    object: &HkxObject,
    member_name: &str,
    expected: usize,
) -> Result<(), CreatureRagdollError> {
    let actual = array_len(object, member_name)?;
    if actual != expected {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!(
                "{}.{} expected {expected} entries, found {actual}",
                object.class_name, member_name
            ),
        });
    }
    Ok(())
}

fn array_len(object: &HkxObject, member_name: &str) -> Result<usize, CreatureRagdollError> {
    let member = object
        .members
        .iter()
        .find(|member| member.name == member_name)
        .ok_or_else(|| CreatureRagdollError::RereadContract {
            reason: format!("{}.{} missing", object.class_name, member_name),
        })?;
    let HkxValue::Array(values) = &member.value else {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!("{}.{} is not an array", object.class_name, member_name),
        });
    };
    Ok(values.len())
}

fn pointer_member(object: &HkxObject, member_name: &str) -> Result<usize, CreatureRagdollError> {
    let member = object
        .members
        .iter()
        .find(|member| member.name == member_name)
        .ok_or_else(|| CreatureRagdollError::RereadContract {
            reason: format!("{}.{} missing", object.class_name, member_name),
        })?;
    let HkxValue::Pointer(Some(index)) = member.value else {
        return Err(CreatureRagdollError::RereadContract {
            reason: format!("{}.{} is null", object.class_name, member_name),
        });
    };
    Ok(index)
}

fn bone_indices(skeleton: &RigSkeletonIr) -> HashMap<&str, usize> {
    skeleton
        .bones
        .iter()
        .enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index))
        .collect()
}

fn member(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn string_member(name: &str, value: &str) -> HkxMember {
    member(
        name,
        HkxValue::String {
            value: value.to_string(),
            is_null: false,
        },
    )
}

fn named_variant(name: &str, class_name: &str, target: usize) -> HkxValue {
    HkxValue::Object(vec![
        string_member("name", name),
        string_member("className", class_name),
        member("variant", HkxValue::Pointer(Some(target))),
    ])
}

fn pointer_array(indices: &[usize]) -> HkxValue {
    HkxValue::Array(
        indices
            .iter()
            .map(|index| HkxValue::Pointer(Some(*index)))
            .collect(),
    )
}

fn qs_value(transform: &QsTransformIr) -> HkxValue {
    HkxValue::F32List(vec![
        transform.translation[0],
        transform.translation[1],
        transform.translation[2],
        0.0,
        transform.rotation[0],
        transform.rotation[1],
        transform.rotation[2],
        transform.rotation[3],
        transform.scale[0],
        transform.scale[1],
        transform.scale[2],
        0.0,
    ])
}

fn matrix_value(transform: &QsTransformIr) -> HkxValue {
    let [x, y, z, w] = normalized_quaternion(transform.rotation);
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let wx = w * x;
    let wy = w * y;
    let wz = w * z;
    HkxValue::F32List(vec![
        1.0 - 2.0 * (yy + zz),
        2.0 * (xy + wz),
        2.0 * (xz - wy),
        0.0,
        2.0 * (xy - wz),
        1.0 - 2.0 * (xx + zz),
        2.0 * (yz + wx),
        0.0,
        2.0 * (xz + wy),
        2.0 * (yz - wx),
        1.0 - 2.0 * (xx + yy),
        0.0,
        transform.translation[0],
        transform.translation[1],
        transform.translation[2],
        1.0,
    ])
}

fn normalized_quaternion(rotation: [f32; 4]) -> [f32; 4] {
    let inverse_length = rotation
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt()
        .recip();
    rotation.map(|value| value * inverse_length)
}

fn inverse_qs_transform(transform: &QsTransformIr) -> QsTransformIr {
    let [x, y, z, w] = normalized_quaternion(transform.rotation);
    let inverse_rotation = [-x, -y, -z, w];
    let rotated_translation =
        rotate_vector(inverse_rotation, transform.translation.map(|value| -value));
    let inverse_scale = transform.scale.map(f32::recip);
    QsTransformIr {
        translation: [
            rotated_translation[0] * inverse_scale[0],
            rotated_translation[1] * inverse_scale[1],
            rotated_translation[2] * inverse_scale[2],
        ],
        rotation: inverse_rotation,
        scale: inverse_scale,
    }
}

fn rotate_vector(rotation: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = rotation;
    let uv = [
        y * vector[2] - z * vector[1],
        z * vector[0] - x * vector[2],
        x * vector[1] - y * vector[0],
    ];
    let uuv = [
        y * uv[2] - z * uv[1],
        z * uv[0] - x * uv[2],
        x * uv[1] - y * uv[0],
    ];
    [
        vector[0] + 2.0 * (w * uv[0] + uuv[0]),
        vector[1] + 2.0 * (w * uv[1] + uuv[1]),
        vector[2] + 2.0 * (w * uv[2] + uuv[2]),
    ]
}

fn transform_point(transform: &QsTransformIr, point: [f32; 3]) -> [f32; 3] {
    let scaled = [
        point[0] * transform.scale[0],
        point[1] * transform.scale[1],
        point[2] * transform.scale[2],
    ];
    let rotated = rotate_vector(normalized_quaternion(transform.rotation), scaled);
    [
        rotated[0] + transform.translation[0],
        rotated[1] + transform.translation[1],
        rotated[2] + transform.translation[2],
    ]
}

fn vector4(vector: [f32; 3], w: f32) -> HkxValue {
    HkxValue::F32List(vec![vector[0], vector[1], vector[2], w])
}

fn aabb(vertices: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in vertices {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex[axis]);
            maximum[axis] = maximum[axis].max(vertex[axis]);
        }
    }
    let center = [
        (minimum[0] + maximum[0]) * 0.5,
        (minimum[1] + maximum[1]) * 0.5,
        (minimum[2] + maximum[2]) * 0.5,
    ];
    let half_extents = [
        (maximum[0] - minimum[0]) * 0.5,
        (maximum[1] - minimum[1]) * 0.5,
        (maximum[2] - minimum[2]) * 0.5,
    ];
    (center, half_extents)
}

fn body_shape_radius(body: &RigidBodyIr) -> f32 {
    let mut vertices = Vec::new();
    collect_shape_vertices(&body.shape, &mut vertices);
    vertices
        .iter()
        .map(|vertex| {
            let delta = [
                vertex[0] - body.mass_properties.center_of_mass[0],
                vertex[1] - body.mass_properties.center_of_mass[1],
                vertex[2] - body.mass_properties.center_of_mass[2],
            ];
            delta.iter().map(|value| value * value).sum::<f32>().sqrt()
        })
        .fold(0.0, f32::max)
        + shape_convex_radius(&body.shape)
}

fn box_hull(half_extents: [f32; 3], convex_radius: f32) -> ConvexHullIr {
    let [x, y, z] = half_extents;
    ConvexHullIr {
        vertices: vec![
            [-x, -y, -z],
            [-x, -y, z],
            [-x, y, -z],
            [-x, y, z],
            [x, -y, -z],
            [x, -y, z],
            [x, y, -z],
            [x, y, z],
        ],
        planes: vec![
            [1.0, 0.0, 0.0, -x],
            [-1.0, 0.0, 0.0, -x],
            [0.0, 1.0, 0.0, -y],
            [0.0, -1.0, 0.0, -y],
            [0.0, 0.0, 1.0, -z],
            [0.0, 0.0, -1.0, -z],
        ],
        convex_radius,
    }
}

fn sampled_sphere(center: [f32; 3], radius: f32, radial_segments: usize) -> Vec<[f32; 3]> {
    let latitude_segments = (radial_segments / 2).max(3);
    let mut vertices = Vec::with_capacity(2 + radial_segments * (latitude_segments - 1));
    vertices.push([center[0], center[1], center[2] - radius]);
    for latitude in 1..latitude_segments {
        let phi = -std::f32::consts::FRAC_PI_2
            + std::f32::consts::PI * latitude as f32 / latitude_segments as f32;
        let ring_radius = radius * phi.cos();
        let z = center[2] + radius * phi.sin();
        for longitude in 0..radial_segments {
            let theta = std::f32::consts::TAU * longitude as f32 / radial_segments as f32;
            vertices.push([
                center[0] + ring_radius * theta.cos(),
                center[1] + ring_radius * theta.sin(),
                z,
            ]);
        }
    }
    vertices.push([center[0], center[1], center[2] + radius]);
    vertices
}

fn sampled_primitive_hull(
    body: &str,
    shape: &str,
    vertices: &[[f32; 3]],
) -> Result<ConvexHullIr, CreatureRagdollError> {
    let topology = crate::collision::hull::compute_hull_topology(vertices).map_err(|error| {
        CreatureRagdollError::PrimitiveConvexification {
            body: body.to_string(),
            shape: shape.to_string(),
            reason: error.to_string(),
        }
    })?;
    Ok(ConvexHullIr {
        vertices: topology.vertices,
        planes: topology.planes,
        convex_radius: 0.0,
    })
}

struct SkyrimRawSource<'a> {
    source: &'a HkxFile,
    data_offset: usize,
}

impl<'a> SkyrimRawSource<'a> {
    fn new(source: &'a HkxFile) -> Result<Self, CreatureRagdollError> {
        let data_offset = source
            .packfile()
            .section("__data__")
            .map(|section| section.offset)
            .ok_or_else(|| CreatureRagdollError::SkyrimSourceLayout {
                context: "packfile".to_string(),
                reason: "missing __data__ section".to_string(),
            })?;
        Ok(Self {
            source,
            data_offset,
        })
    }

    fn object_pointer(&self, source_absolute: usize) -> Result<usize, CreatureRagdollError> {
        let source_relative = self.relative_offset(source_absolute)?;
        let fixup = self
            .source
            .packfile()
            .global_fixups
            .iter()
            .find(|fixup| fixup.source as usize == source_relative)
            .ok_or_else(|| CreatureRagdollError::SkyrimSourceFixupMissing {
                kind: "global".to_string(),
                offset: source_absolute,
            })?;
        let target_section = self
            .source
            .packfile()
            .sections
            .get(fixup.section as usize)
            .ok_or_else(|| CreatureRagdollError::SkyrimSourceLayout {
                context: format!("global fixup at {source_absolute}"),
                reason: format!("invalid target section {}", fixup.section),
            })?;
        let target_absolute = target_section.offset + fixup.target as usize;
        self.source
            .objects()
            .iter()
            .position(|object| object.offset == target_absolute)
            .ok_or(CreatureRagdollError::SkyrimSourceObjectMissing {
                offset: target_absolute,
            })
    }

    fn local_target(&self, source_absolute: usize) -> Result<usize, CreatureRagdollError> {
        let source_relative = self.relative_offset(source_absolute)?;
        self.source
            .packfile()
            .local_fixups
            .iter()
            .find(|fixup| fixup.source as usize == source_relative)
            .map(|fixup| self.data_offset + fixup.target as usize)
            .ok_or_else(|| CreatureRagdollError::SkyrimSourceFixupMissing {
                kind: "local".to_string(),
                offset: source_absolute,
            })
    }

    fn pointer_array(&self, header_absolute: usize) -> Result<Vec<usize>, CreatureRagdollError> {
        let length = self.u32(header_absolute + 8)? as usize;
        if length == 0 {
            return Ok(Vec::new());
        }
        let payload = self.local_target(header_absolute)?;
        (0..length)
            .map(|index| self.object_pointer(payload + index * 8))
            .collect()
    }

    fn i32_array(&self, header_absolute: usize) -> Result<Vec<i32>, CreatureRagdollError> {
        let length = self.u32(header_absolute + 8)? as usize;
        if length == 0 {
            return Ok(Vec::new());
        }
        let payload = self.local_target(header_absolute)?;
        (0..length)
            .map(|index| self.i32(payload + index * 4))
            .collect()
    }

    fn string(&self, pointer_absolute: usize) -> Result<Option<String>, CreatureRagdollError> {
        let Ok(start) = self.local_target(pointer_absolute) else {
            return Ok(None);
        };
        let bytes = self.source.source_bytes();
        let Some(relative_end) = bytes
            .get(start..)
            .and_then(|tail| tail.iter().position(|byte| *byte == 0))
        else {
            return Err(CreatureRagdollError::SkyrimSourceLayout {
                context: format!("string pointer at {pointer_absolute}"),
                reason: "unterminated string".to_string(),
            });
        };
        let value = std::str::from_utf8(&bytes[start..start + relative_end]).map_err(|error| {
            CreatureRagdollError::SkyrimSourceLayout {
                context: format!("string pointer at {pointer_absolute}"),
                reason: error.to_string(),
            }
        })?;
        Ok(Some(value.to_string()))
    }

    fn f32s<const N: usize>(&self, offset: usize) -> Result<[f32; N], CreatureRagdollError> {
        let mut values = [0.0; N];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.f32(offset + index * 4)?;
        }
        Ok(values)
    }

    fn u16(&self, offset: usize) -> Result<u16, CreatureRagdollError> {
        Ok(u16::from_le_bytes(self.bytes::<2>(offset)?))
    }

    fn i16(&self, offset: usize) -> Result<i16, CreatureRagdollError> {
        Ok(i16::from_le_bytes(self.bytes::<2>(offset)?))
    }

    fn u32(&self, offset: usize) -> Result<u32, CreatureRagdollError> {
        Ok(u32::from_le_bytes(self.bytes::<4>(offset)?))
    }

    fn i32(&self, offset: usize) -> Result<i32, CreatureRagdollError> {
        Ok(i32::from_le_bytes(self.bytes::<4>(offset)?))
    }

    fn f32(&self, offset: usize) -> Result<f32, CreatureRagdollError> {
        Ok(f32::from_le_bytes(self.bytes::<4>(offset)?))
    }

    fn bytes<const N: usize>(&self, offset: usize) -> Result<[u8; N], CreatureRagdollError> {
        let end =
            offset
                .checked_add(N)
                .ok_or_else(|| CreatureRagdollError::SkyrimSourceLayout {
                    context: format!("raw read at {offset}"),
                    reason: "offset overflow".to_string(),
                })?;
        self.source
            .source_bytes()
            .get(offset..end)
            .ok_or_else(|| CreatureRagdollError::SkyrimSourceLayout {
                context: format!("raw read at {offset}"),
                reason: format!("needs {N} bytes"),
            })?
            .try_into()
            .map_err(|_| CreatureRagdollError::SkyrimSourceLayout {
                context: format!("raw read at {offset}"),
                reason: format!("could not read {N} bytes"),
            })
    }

    fn relative_offset(&self, absolute: usize) -> Result<usize, CreatureRagdollError> {
        absolute.checked_sub(self.data_offset).ok_or_else(|| {
            CreatureRagdollError::SkyrimSourceLayout {
                context: format!("absolute offset {absolute}"),
                reason: format!("precedes data section at {}", self.data_offset),
            }
        })
    }
}

fn unique_source_object_index(
    source: &HkxFile,
    class_name: &str,
) -> Result<usize, CreatureRagdollError> {
    let indices: Vec<_> = source
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(index, object)| (object.class_name == class_name).then_some(index))
        .collect();
    if indices.len() != 1 {
        return Err(CreatureRagdollError::SkyrimSourceClassCount {
            class_name: class_name.to_string(),
            expected: 1,
            actual: indices.len(),
        });
    }
    Ok(indices[0])
}

fn parse_source_skeleton(object: &HkxObject) -> Result<RigSkeletonIr, CreatureRagdollError> {
    let context = format!("hkaSkeleton at {}", object.offset);
    let name = source_string_member(object, "name", &context)?;
    let parents = source_array_member(object, "parentIndices", &context)?;
    let source_bones = source_array_member(object, "bones", &context)?;
    let poses = source_array_member(object, "referencePose", &context)?;
    if parents.len() != source_bones.len() || poses.len() != source_bones.len() {
        return Err(CreatureRagdollError::SkyrimSourceLayout {
            context,
            reason: format!(
                "{} parents, {} bones, {} poses",
                parents.len(),
                source_bones.len(),
                poses.len()
            ),
        });
    }
    let bones = source_bones
        .iter()
        .enumerate()
        .map(|(index, source_bone)| {
            let members = source_bone.as_object_members().ok_or_else(|| {
                CreatureRagdollError::SkyrimSourceLayout {
                    context: format!("{context}.bones[{index}]"),
                    reason: "bone is not an inline object".to_string(),
                }
            })?;
            let bone_object = HkxObject {
                name: None,
                offset: 0,
                signature: 0,
                class_name: "hkaBone".to_string(),
                members: members.to_vec(),
            };
            let parent = match parents[index] {
                HkxValue::I16(-1) => None,
                HkxValue::I16(parent) if parent >= 0 => Some(parent as usize),
                ref value => {
                    return Err(CreatureRagdollError::SkyrimSourceLayout {
                        context: format!("{context}.parentIndices[{index}]"),
                        reason: format!("expected i16 parent, found {}", value.variant_name()),
                    });
                }
            };
            let HkxValue::F32List(values) = &poses[index] else {
                return Err(CreatureRagdollError::SkyrimSourceLayout {
                    context: format!("{context}.referencePose[{index}]"),
                    reason: "expected hkQsTransform".to_string(),
                });
            };
            Ok(RigBoneIr {
                name: source_string_member(&bone_object, "name", &context)?,
                parent,
                reference_pose: qs_from_slice(
                    values,
                    &format!("{context}.referencePose[{index}]"),
                )?,
                lock_translation: source_bool_member(&bone_object, "lockTranslation", &context)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RigSkeletonIr { name, bones })
}

fn parse_source_mapper(
    raw: &SkyrimRawSource<'_>,
    object: &HkxObject,
    ragdoll: &RigSkeletonIr,
    animation: &RigSkeletonIr,
) -> Result<Vec<BoneMappingIr>, CreatureRagdollError> {
    let header = object.offset + 32;
    let length = raw.u32(header + 8)? as usize;
    let payload = raw.local_target(header)?;
    let source_mappings = (0..length)
        .map(|index| {
            let entry = payload + index * 64;
            let bone_a = raw.i16(entry)?;
            let bone_b = raw.i16(entry + 2)?;
            let ragdoll_bone = ragdoll.bones.get(bone_a as usize).ok_or_else(|| {
                CreatureRagdollError::SkyrimSourceLayout {
                    context: format!("hkaSkeletonMapper.simpleMappings[{index}]"),
                    reason: format!("ragdoll bone index {bone_a} is out of range"),
                }
            })?;
            let animation_bone = animation.bones.get(bone_b as usize).ok_or_else(|| {
                CreatureRagdollError::SkyrimSourceLayout {
                    context: format!("hkaSkeletonMapper.simpleMappings[{index}]"),
                    reason: format!("animation bone index {bone_b} is out of range"),
                }
            })?;
            let transform = raw.f32s::<12>(entry + 16)?;
            Ok(BoneMappingIr {
                ragdoll_bone: ragdoll_bone.name.clone(),
                animation_bone: animation_bone.name.clone(),
                ragdoll_from_animation: qs_from_slice(
                    &transform,
                    &format!("hkaSkeletonMapper.simpleMappings[{index}]"),
                )?,
            })
        })
        .collect::<Result<Vec<_>, CreatureRagdollError>>()?;
    let mut mappings: Vec<BoneMappingIr> = Vec::with_capacity(source_mappings.len());
    for mapping in source_mappings {
        if let Some(previous) = mappings
            .iter()
            .find(|previous| previous.ragdoll_bone == mapping.ragdoll_bone)
        {
            if previous == &mapping {
                continue;
            }
            return Err(CreatureRagdollError::SkyrimSourceBoneBodyMap {
                reason: format!(
                    "ragdoll bone {} maps to both {} and {}",
                    mapping.ragdoll_bone, previous.animation_bone, mapping.animation_bone
                ),
            });
        }
        mappings.push(mapping);
    }
    Ok(mappings)
}

fn parse_skyrim_source_body(
    raw: &SkyrimRawSource<'_>,
    object: &HkxObject,
    ragdoll_bone: &str,
) -> Result<RigidBodyIr, CreatureRagdollError> {
    let name = raw
        .string(object.offset + 0xb0)?
        .unwrap_or_else(|| format!("RigidBody@{}", object.offset));
    let shape_index = raw.object_pointer(object.offset + 0x20)?;
    let shape = parse_skyrim_source_shape(raw, shape_index, &name, &mut HashSet::new())?;
    let inverse = raw.f32s::<4>(object.offset + 0x220)?;
    if inverse
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(CreatureRagdollError::SkyrimSourceLayout {
            context: format!("hkpRigidBody {name}.inertiaAndMassInv"),
            reason: format!("expected four finite positive inverse values, found {inverse:?}"),
        });
    }
    Ok(RigidBodyIr {
        name,
        ragdoll_bone: ragdoll_bone.to_string(),
        shape,
        world_from_body: qs_from_matrix(raw.f32s::<16>(object.offset + 0x170)?),
        mass_properties: MassPropertiesIr {
            mass: inverse[3].recip(),
            center_of_mass: first_three(raw.f32s::<4>(object.offset + 0x1f0)?),
            inertia_diagonal: [inverse[0].recip(), inverse[1].recip(), inverse[2].recip()],
        },
        friction: raw.f32(object.offset + 0xd4)?,
        restitution: raw.f32(object.offset + 0xd8)?,
        linear_damping: half_to_f32(raw.u16(object.offset + 0x214)?),
        angular_damping: half_to_f32(raw.u16(object.offset + 0x216)?),
        collision_filter_info: raw.u32(object.offset + 0x4c)?,
    })
}

fn parse_skyrim_source_shape(
    raw: &SkyrimRawSource<'_>,
    shape_index: usize,
    body_name: &str,
    visited: &mut HashSet<usize>,
) -> Result<RagdollShapeIr, CreatureRagdollError> {
    if !visited.insert(shape_index) {
        return Err(CreatureRagdollError::SkyrimSourceLayout {
            context: format!("hkpRigidBody {body_name}.shape"),
            reason: format!("shape graph contains a cycle at object {shape_index}"),
        });
    }
    let shape_object = &raw.source.objects()[shape_index];
    let shape = match shape_object.class_name.as_str() {
        "hkpCapsuleShape" => RagdollShapeIr::Capsule {
            radius: raw.f32(shape_object.offset + 0x20)?,
            vertex_a: first_three(raw.f32s::<4>(shape_object.offset + 0x30)?),
            vertex_b: first_three(raw.f32s::<4>(shape_object.offset + 0x40)?),
        },
        "hkpSphereShape" => RagdollShapeIr::Sphere {
            center: [0.0; 3],
            radius: raw.f32(shape_object.offset + 0x20)?,
        },
        "hkpBoxShape" => RagdollShapeIr::Box {
            half_extents: first_three(raw.f32s::<4>(shape_object.offset + 0x30)?),
            convex_radius: raw.f32(shape_object.offset + 0x20)?,
        },
        "hkpConvexVerticesShape" => {
            RagdollShapeIr::ConvexHull(parse_skyrim_source_convex_hull(raw, shape_object)?)
        }
        "hkpConvexTranslateShape" => {
            let child = raw.object_pointer(shape_object.offset + 0x30)?;
            let translation = first_three(raw.f32s::<4>(shape_object.offset + 0x40)?);
            let child_shape = parse_skyrim_source_shape(raw, child, body_name, visited)?;
            transform_source_shape(
                child_shape,
                &QsTransformIr {
                    translation,
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            )?
        }
        "hkpConvexTransformShape" => {
            let child = raw.object_pointer(shape_object.offset + 0x30)?;
            let transform = qs_from_slice(
                &raw.f32s::<12>(shape_object.offset + 0x40)?,
                &format!("hkpRigidBody {body_name}.hkpConvexTransformShape.transform"),
            )?;
            let child_shape = parse_skyrim_source_shape(raw, child, body_name, visited)?;
            transform_source_shape(child_shape, &transform)?
        }
        class_name => {
            return Err(CreatureRagdollError::UnsupportedSkyrimSourceShape {
                body: body_name.to_string(),
                class_name: class_name.to_string(),
            });
        }
    };
    visited.remove(&shape_index);
    Ok(shape)
}

fn transform_source_shape(
    shape: RagdollShapeIr,
    transform: &QsTransformIr,
) -> Result<RagdollShapeIr, CreatureRagdollError> {
    let scale = transform.scale[0];
    if !scale.is_finite()
        || scale <= 0.0
        || transform
            .scale
            .iter()
            .any(|component| (*component - scale).abs() > 1.0e-4)
    {
        return Err(CreatureRagdollError::SkyrimSourceLayout {
            context: "source shape transform".to_string(),
            reason: format!(
                "expected finite positive uniform scale, found {:?}",
                transform.scale
            ),
        });
    }
    Ok(match shape {
        RagdollShapeIr::Capsule {
            vertex_a,
            vertex_b,
            radius,
        } => RagdollShapeIr::Capsule {
            vertex_a: transform_point(transform, vertex_a),
            vertex_b: transform_point(transform, vertex_b),
            radius: radius * scale,
        },
        RagdollShapeIr::Sphere { center, radius } => RagdollShapeIr::Sphere {
            center: transform_point(transform, center),
            radius: radius * scale,
        },
        RagdollShapeIr::Box {
            half_extents,
            convex_radius,
        } => RagdollShapeIr::ConvexHull(transform_source_hull(
            box_hull(half_extents, convex_radius),
            transform,
        )),
        RagdollShapeIr::ConvexHull(hull) => {
            RagdollShapeIr::ConvexHull(transform_source_hull(hull, transform))
        }
        RagdollShapeIr::Compound { children } => RagdollShapeIr::Compound {
            children: children
                .into_iter()
                .map(|child| transform_source_shape(child, transform))
                .collect::<Result<Vec<_>, _>>()?,
        },
    })
}

fn transform_source_hull(hull: ConvexHullIr, transform: &QsTransformIr) -> ConvexHullIr {
    let scale = transform.scale[0];
    let rotation = normalized_quaternion(transform.rotation);
    ConvexHullIr {
        vertices: hull
            .vertices
            .into_iter()
            .map(|vertex| transform_point(transform, vertex))
            .collect(),
        planes: hull
            .planes
            .into_iter()
            .map(|plane| {
                let normal = rotate_vector(rotation, [plane[0], plane[1], plane[2]]);
                [
                    normal[0],
                    normal[1],
                    normal[2],
                    plane[3] * scale
                        - normal
                            .iter()
                            .zip(transform.translation)
                            .map(|(left, right)| left * right)
                            .sum::<f32>(),
                ]
            })
            .collect(),
        convex_radius: hull.convex_radius * scale,
    }
}

fn parse_skyrim_source_convex_hull(
    raw: &SkyrimRawSource<'_>,
    object: &HkxObject,
) -> Result<ConvexHullIr, CreatureRagdollError> {
    let group_header = object.offset + 0x50;
    let group_count = raw.u32(group_header + 8)? as usize;
    let payload = raw.local_target(group_header)?;
    let vertex_count = raw.u32(object.offset + 0x60)? as usize;
    let mut vertices = Vec::with_capacity(vertex_count);
    for group in 0..group_count {
        let values = raw.f32s::<12>(payload + group * 48)?;
        for lane in 0..4 {
            if vertices.len() == vertex_count {
                break;
            }
            vertices.push([values[lane], values[4 + lane], values[8 + lane]]);
        }
    }
    let plane_header = object.offset + 0x78;
    let plane_count = raw.u32(plane_header + 8)? as usize;
    let plane_payload = raw.local_target(plane_header)?;
    let planes = (0..plane_count)
        .map(|index| raw.f32s::<4>(plane_payload + index * 16))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ConvexHullIr {
        vertices,
        planes,
        convex_radius: raw.f32(object.offset + 0x20)?,
    })
}

fn parse_skyrim_source_constraint(
    raw: &SkyrimRawSource<'_>,
    object: &HkxObject,
    ordinal: usize,
    body_name_by_object: &HashMap<usize, String>,
) -> Result<ConstraintIr, CreatureRagdollError> {
    let data_index = raw.object_pointer(object.offset + 24)?;
    let body_a_index = raw.object_pointer(object.offset + 40)?;
    let body_b_index = raw.object_pointer(object.offset + 48)?;
    let body_a = body_name_by_object.get(&body_a_index).ok_or_else(|| {
        CreatureRagdollError::SkyrimSourceLayout {
            context: format!("hkpConstraintInstance[{ordinal}].entityA"),
            reason: "entity is outside the hkaRagdollInstance body closure".to_string(),
        }
    })?;
    let body_b = body_name_by_object.get(&body_b_index).ok_or_else(|| {
        CreatureRagdollError::SkyrimSourceLayout {
            context: format!("hkpConstraintInstance[{ordinal}].entityB"),
            reason: "entity is outside the hkaRagdollInstance body closure".to_string(),
        }
    })?;
    let data = &raw.source.objects()[data_index];
    let name = raw.string(object.offset + 80)?.unwrap_or_else(|| {
        format!(
            "{}:{}-{}",
            data.class_name,
            body_a.as_str(),
            body_b.as_str()
        )
    });
    let atoms = data.offset + 0x20;
    let frame_a = qs_from_matrix(raw.f32s::<16>(atoms + 0x10)?);
    let frame_b = qs_from_matrix(raw.f32s::<16>(atoms + 0x50)?);
    let kind = match data.class_name.as_str() {
        "hkpLimitedHingeConstraintData" => ConstraintKindIr::LimitedHinge(LimitedHingeIr {
            frame_a,
            frame_b,
            max_friction_torque: raw.f32(atoms + 0xc0)?,
            min_angle: raw.f32(atoms + 0xc8)?,
            max_angle: raw.f32(atoms + 0xcc)?,
        }),
        "hkpRagdollConstraintData" => ConstraintKindIr::Ragdoll(RagdollLimitsIr {
            frame_a,
            frame_b,
            max_friction_torque: raw.f32(atoms + 0x108)?,
            twist_min: raw.f32(atoms + 0x114)?,
            twist_max: raw.f32(atoms + 0x118)?,
            cone_limit: raw.f32(atoms + 0x12c)?,
            plane_min: raw.f32(atoms + 0x13c)?,
            plane_max: raw.f32(atoms + 0x140)?,
        }),
        class_name => {
            return Err(CreatureRagdollError::UnsupportedSkyrimSourceConstraint {
                constraint: name,
                class_name: class_name.to_string(),
            });
        }
    };
    Ok(ConstraintIr {
        name,
        body_a: body_a.clone(),
        body_b: body_b.clone(),
        kind,
    })
}

fn source_member<'a>(
    object: &'a HkxObject,
    name: &str,
    context: &str,
) -> Result<&'a HkxValue, CreatureRagdollError> {
    object
        .members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
        .ok_or_else(|| CreatureRagdollError::SkyrimSourceLayout {
            context: context.to_string(),
            reason: format!("missing member {name}"),
        })
}

fn source_string_member(
    object: &HkxObject,
    name: &str,
    context: &str,
) -> Result<String, CreatureRagdollError> {
    match source_member(object, name, context)? {
        HkxValue::String {
            value,
            is_null: false,
        } => Ok(value.clone()),
        value => Err(CreatureRagdollError::SkyrimSourceLayout {
            context: context.to_string(),
            reason: format!(
                "member {name} expected string, found {}",
                value.variant_name()
            ),
        }),
    }
}

fn source_bool_member(
    object: &HkxObject,
    name: &str,
    context: &str,
) -> Result<bool, CreatureRagdollError> {
    match source_member(object, name, context)? {
        HkxValue::Bool(value) => Ok(*value),
        value => Err(CreatureRagdollError::SkyrimSourceLayout {
            context: context.to_string(),
            reason: format!(
                "member {name} expected bool, found {}",
                value.variant_name()
            ),
        }),
    }
}

fn source_array_member<'a>(
    object: &'a HkxObject,
    name: &str,
    context: &str,
) -> Result<&'a [HkxValue], CreatureRagdollError> {
    match source_member(object, name, context)? {
        HkxValue::Array(values) => Ok(values),
        value => Err(CreatureRagdollError::SkyrimSourceLayout {
            context: context.to_string(),
            reason: format!(
                "member {name} expected array, found {}",
                value.variant_name()
            ),
        }),
    }
}

fn qs_from_slice(values: &[f32], context: &str) -> Result<QsTransformIr, CreatureRagdollError> {
    if values.len() != 12 {
        return Err(CreatureRagdollError::SkyrimSourceLayout {
            context: context.to_string(),
            reason: format!("hkQsTransform needs 12 floats, found {}", values.len()),
        });
    }
    Ok(QsTransformIr {
        translation: [values[0], values[1], values[2]],
        rotation: [values[4], values[5], values[6], values[7]],
        scale: [values[8], values[9], values[10]],
    })
}

fn qs_from_matrix(values: [f32; 16]) -> QsTransformIr {
    let m00 = values[0];
    let m01 = values[4];
    let m02 = values[8];
    let m10 = values[1];
    let m11 = values[5];
    let m12 = values[9];
    let m20 = values[2];
    let m21 = values[6];
    let m22 = values[10];
    let trace = m00 + m11 + m22;
    let rotation = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
    };
    QsTransformIr {
        translation: [values[12], values[13], values[14]],
        rotation: normalized_quaternion(rotation),
        scale: [1.0; 3],
    }
}

fn first_three(values: [f32; 4]) -> [f32; 3] {
    [values[0], values[1], values[2]]
}
