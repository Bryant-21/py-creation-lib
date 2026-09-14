use std::collections::{HashMap, HashSet};

use havok_native::collision::capsule::build_fo4_capsule_collision;
use havok_native::collision::compressed_mesh::BuildOptions;
use havok_native::collision::sphere::build_fo4_sphere_collision;
use havok_native::convert::creature_ragdoll::{
    BoneMappingIr, ConstraintIr, ConstraintKindIr, ConvexHullIr, CreatureRagdollError,
    CreatureRagdollIr, HingeIr, LimitedHingeIr, MassPropertiesIr, PrismaticIr, QsTransformIr,
    RagdollLimitsIr, RagdollShapeIr, RigBoneIr, RigSkeletonIr, RigidBodyIr,
    reconstruct_fo4_embedded_creature_ragdoll, validate_creature_ragdoll_ir,
};
use indexmap::IndexMap;
use thiserror::Error;

use crate::model::{NifBlock, NifFile, NifValue};

pub const LEGACY_BHK_TO_FO4_CHARACTER_SCALE: f32 = 6.999_125;

#[derive(Debug, Error, PartialEq)]
pub enum LegacyCreatureRagdollError {
    #[error(
        "unsupported legacy creature NIF header: version {version:?}, user {user_version}, Bethesda {bs_version}"
    )]
    UnsupportedHeader {
        version: (u8, u8, u8, u8),
        user_version: u32,
        bs_version: u32,
    },
    #[error("legacy creature NIF contains no articulated rigid-body closure")]
    MissingRagdoll,
    #[error("block {block} is missing required field {field}")]
    MissingField { block: usize, field: String },
    #[error("block {block} field {field} references invalid block {target}")]
    InvalidReference {
        block: usize,
        field: String,
        target: i32,
    },
    #[error("node {node} has more than one parent")]
    MultipleNodeParents { node: usize },
    #[error("node hierarchy contains a cycle at block {node}")]
    NodeCycle { node: usize },
    #[error("rigid body {body} is linked to more than one target node")]
    DuplicateBodyTarget { body: usize },
    #[error("target node {node} is outside animation root {root}")]
    TargetOutsideAnimationRoot { node: usize, root: usize },
    #[error("animation node {node} has no unique non-empty name")]
    InvalidBoneName { node: usize },
    #[error("legacy ragdoll has {components} articulated components; exactly one is required")]
    MultipleComponents { components: usize },
    #[error("legacy ragdoll is not a tree: {reason}")]
    NonTree { reason: String },
    #[error("rigid body {body} uses unsupported source shape {shape}")]
    UnsupportedShape { body: usize, shape: String },
    #[error("rigid body {body} uses compound source shape {shape}")]
    UnsupportedCompoundShape { body: usize, shape: String },
    #[error("constraint block {constraint} uses unsupported source semantic {kind}")]
    UnsupportedConstraint { constraint: usize, kind: String },
    #[error("rigid body {body} has material off-diagonal inertia at [{row},{column}]")]
    OffDiagonalInertia {
        body: usize,
        row: usize,
        column: usize,
    },
    #[error("constraint block {constraint} has a degenerate {frame} frame")]
    DegenerateConstraintFrame { constraint: usize, frame: String },
    #[error(
        "legacy ragdoll with {bodies} bodies needs {required_ids} collision-filter IDs; hkpGroupFilter has 32"
    )]
    CollisionFilterIdExhausted { bodies: usize, required_ids: usize },
    #[error("legacy ragdoll value {path} is missing, invalid, or non-finite")]
    InvalidValue { path: String },
    #[error(transparent)]
    InvalidIr(#[from] CreatureRagdollError),
    #[error("could not install FO4 creature collision: {reason}")]
    Install { reason: String },
}

#[derive(Clone, Copy, Debug)]
struct Transform {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: f32,
}

impl Transform {
    fn identity() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
        }
    }

    fn combine(self, local: Self) -> Self {
        Self {
            translation: add(
                self.translation,
                rotate(self.rotation, scale3(local.translation, self.scale)),
            ),
            rotation: normalize_quaternion(quat_mul(self.rotation, local.rotation))
                .unwrap_or([0.0, 0.0, 0.0, 1.0]),
            scale: self.scale * local.scale,
        }
    }

    fn rigid(self) -> Self {
        Self { scale: 1.0, ..self }
    }

    fn relative_to(self, world: Self) -> Self {
        let inverse_rotation = quat_conjugate(self.rotation);
        let scale = world.scale / self.scale;
        Self {
            translation: scale3(
                rotate(inverse_rotation, sub(world.translation, self.translation)),
                self.scale.recip(),
            ),
            rotation: normalize_quaternion(quat_mul(inverse_rotation, world.rotation))
                .unwrap_or([0.0, 0.0, 0.0, 1.0]),
            scale,
        }
    }

    fn qs(self) -> QsTransformIr {
        QsTransformIr {
            translation: self.translation,
            rotation: self.rotation,
            scale: [self.scale; 3],
        }
    }
}

#[derive(Debug)]
struct SourceConstraint {
    block_id: usize,
    body_a: usize,
    body_b: usize,
    kind: ConstraintKindIr,
}

pub fn extract_fnv_fo3_creature_ragdoll(
    nif: &NifFile,
) -> Result<CreatureRagdollIr, LegacyCreatureRagdollError> {
    if nif.header.version != (20, 2, 0, 7)
        || nif.header.user_version != 11
        || nif.header.bs_version != 34
    {
        return Err(LegacyCreatureRagdollError::UnsupportedHeader {
            version: nif.header.version,
            user_version: nif.header.user_version,
            bs_version: nif.header.bs_version,
        });
    }

    let node_ids = nif
        .find_blocks("NiNode")
        .into_iter()
        .collect::<HashSet<_>>();
    let parent_of_node = node_parents(nif, &node_ids)?;
    let body_targets = body_targets(nif, &node_ids)?;
    let constraints = source_constraints(nif, &body_targets)?;
    if constraints.is_empty() {
        return Err(LegacyCreatureRagdollError::MissingRagdoll);
    }

    let body_ids = constraints
        .iter()
        .flat_map(|constraint| [constraint.body_a, constraint.body_b])
        .collect::<HashSet<_>>();
    let components = connected_components(&body_ids, &constraints);
    if components.len() != 1 {
        return Err(LegacyCreatureRagdollError::MultipleComponents {
            components: components.len(),
        });
    }
    let (root_body, parent_of_body, body_order) = body_tree(&body_ids, &constraints)?;
    let filter_ids = allocate_filter_ids(&body_order, &parent_of_body)?;
    let first_target = body_targets[&root_body];
    let animation_root = animation_root(first_target, nif, &parent_of_node);
    let (animation_skeleton, animation_world) =
        animation_skeleton(nif, animation_root, &node_ids, &body_targets, &body_ids)?;

    let mut body_world = HashMap::new();
    for body_id in &body_order {
        let target = body_targets[body_id];
        let target_world = animation_world[&target];
        let body = &nif.blocks[*body_id];
        let world = if body.type_name == "bhkRigidBodyT" {
            target_world.combine(body_local_transform(body)?).rigid()
        } else {
            target_world.rigid()
        };
        body_world.insert(*body_id, world);
    }

    let body_order_indices = body_order
        .iter()
        .enumerate()
        .map(|(index, body)| (*body, index))
        .collect::<HashMap<_, _>>();
    let mut ragdoll_bones = Vec::with_capacity(body_order.len());
    let mut bodies = Vec::with_capacity(body_order.len());
    let mut mappings = Vec::with_capacity(body_order.len());
    let mut names = HashSet::new();
    let mut body_names = HashMap::new();

    for body_id in &body_order {
        let target = body_targets[body_id];
        let animation_bone = bone_name(&nif.blocks[target])?;
        if !names.insert(animation_bone.clone()) {
            return Err(LegacyCreatureRagdollError::InvalidBoneName { node: target });
        }
        let body_name = format!("{animation_bone}_body");
        body_names.insert(*body_id, body_name.clone());
        let parent_body = parent_of_body.get(body_id).copied();
        let reference_pose = match parent_body {
            Some(parent) => body_world[&parent].relative_to(body_world[body_id]).qs(),
            None => body_world[body_id].qs(),
        };
        ragdoll_bones.push(RigBoneIr {
            name: animation_bone.clone(),
            parent: parent_body.map(|parent| body_order_indices[&parent]),
            reference_pose,
            lock_translation: false,
        });

        let animation_pose = animation_world[&target];
        mappings.push(BoneMappingIr {
            ragdoll_bone: animation_bone.clone(),
            animation_bone: animation_bone.clone(),
            ragdoll_from_animation: body_world[body_id].relative_to(animation_pose).qs(),
        });

        let subsystem_id = filter_ids[body_id];
        let parent_id = parent_body
            .map(|parent| filter_ids[&parent])
            .unwrap_or(subsystem_id);
        bodies.push(parse_body(
            nif,
            *body_id,
            &animation_bone,
            &body_name,
            body_world[body_id],
            hkp_group_filter_info(subsystem_id, parent_id),
        )?);
    }

    let constraints = constraints
        .into_iter()
        .map(|constraint| ConstraintIr {
            name: format!(
                "{}_{}",
                nif.blocks[constraint.block_id].type_name, constraint.block_id
            ),
            body_a: body_names[&constraint.body_a].clone(),
            body_b: body_names[&constraint.body_b].clone(),
            kind: constraint.kind,
        })
        .collect::<Vec<_>>();
    let name = source_name(nif);
    let ir = CreatureRagdollIr {
        name: name.clone(),
        animation_skeleton,
        ragdoll_skeleton: RigSkeletonIr {
            name: format!("{name}_Ragdoll"),
            bones: ragdoll_bones,
        },
        bodies,
        constraints,
        mappings,
    };
    validate_creature_ragdoll_ir(&ir)?;
    Ok(ir)
}

pub fn install_fo4_creature_ragdoll_collision(
    nif: &mut NifFile,
    ir: &CreatureRagdollIr,
) -> Result<usize, LegacyCreatureRagdollError> {
    let embedded = reconstruct_fo4_embedded_creature_ragdoll(ir)?;
    let target_ids = embedded
        .animation_bone_targets
        .iter()
        .map(|name| unique_named_node(nif, name))
        .collect::<Result<Vec<_>, _>>()?;
    for target in &target_ids {
        require_open_collision_slot(nif, *target)?;
    }

    let mut system_fields = IndexMap::new();
    system_fields.insert(
        "Binary Data".to_string(),
        crate::cloth::bytes_to_byte_array(&embedded.binary_data),
    );
    let system_id = nif.add_block("bhkRagdollSystem", Some(system_fields));
    for (body_id, target) in target_ids.into_iter().enumerate() {
        install_np_collision_object(nif, target, system_id, body_id)?;
    }
    Ok(embedded.animation_bone_targets.len())
}

pub fn install_fo4_creature_controller_collision(
    nif: &mut NifFile,
    total_height_havok: f32,
    radius_havok: f32,
    up: [f32; 3],
    collision_filter_info: u32,
) -> Result<(), LegacyCreatureRagdollError> {
    if !total_height_havok.is_finite()
        || !radius_havok.is_finite()
        || total_height_havok < radius_havok * 2.0
        || radius_havok <= 0.0
    {
        return Err(LegacyCreatureRagdollError::Install {
            reason: format!(
                "invalid controller capsule height/radius {total_height_havok}/{radius_havok}"
            ),
        });
    }
    let up_length = up.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !up_length.is_finite() || up_length <= f32::EPSILON {
        return Err(LegacyCreatureRagdollError::Install {
            reason: "controller up axis is invalid".to_string(),
        });
    }
    let up = up.map(|value| value / up_length);
    let half_axis = total_height_havok * 0.5 - radius_havok;
    let a = [
        up[0] * half_axis,
        up[1] * half_axis,
        up[2] * half_axis,
        radius_havok,
    ];
    let b = [
        -up[0] * half_axis,
        -up[1] * half_axis,
        -up[2] * half_axis,
        radius_havok,
    ];
    let layer = ((collision_filter_info & 0xff) as u8).max(1);
    let options = BuildOptions {
        layer,
        ..BuildOptions::default()
    };
    let blob = if half_axis <= f32::EPSILON {
        build_fo4_sphere_collision(radius_havok, [0.0; 3], &options)
    } else {
        let convex_radius = (radius_havok * 0.5).min(0.05);
        build_fo4_capsule_collision(a, b, convex_radius, [0.0; 3], &options)
    }
    .map_err(|error| LegacyCreatureRagdollError::Install {
        reason: format!("build CharacterController collision: {error}"),
    })?;
    let target = ensure_character_controller_node(nif)?;
    require_open_collision_slot(nif, target)?;
    let mut system_fields = IndexMap::new();
    system_fields.insert(
        "Binary Data".to_string(),
        crate::cloth::bytes_to_byte_array(&blob),
    );
    let system_id = nif.add_block("bhkPhysicsSystem", Some(system_fields));
    install_np_collision_object(nif, target, system_id, 0)?;
    Ok(())
}

fn unique_named_node(
    nif: &NifFile,
    expected_name: &str,
) -> Result<usize, LegacyCreatureRagdollError> {
    let matches = named_nodes(nif, expected_name);
    match matches.as_slice() {
        [target] => Ok(*target),
        [] => Err(LegacyCreatureRagdollError::Install {
            reason: format!("visual skeleton has no node named {expected_name:?}"),
        }),
        _ => Err(LegacyCreatureRagdollError::Install {
            reason: format!("visual skeleton has duplicate nodes named {expected_name:?}"),
        }),
    }
}

fn named_nodes(nif: &NifFile, expected_name: &str) -> Vec<usize> {
    nif.blocks
        .iter()
        .filter(|block| {
            matches!(block.type_name.as_str(), "NiNode" | "BSFadeNode")
                && matches!(
                    block.get_field("Name"),
                    Some(NifValue::String(name)) if name.eq_ignore_ascii_case(expected_name)
                )
        })
        .map(|block| block.block_id)
        .collect()
}

fn ensure_character_controller_node(
    nif: &mut NifFile,
) -> Result<usize, LegacyCreatureRagdollError> {
    match named_nodes(nif, "CharacterController").as_slice() {
        [target] => return Ok(*target),
        [] => {}
        _ => {
            return Err(LegacyCreatureRagdollError::Install {
                reason: "visual skeleton has duplicate nodes named \"CharacterController\""
                    .to_string(),
            });
        }
    }
    let roots = nif
        .header
        .footer_roots
        .iter()
        .filter_map(|root| usize::try_from(*root).ok())
        .filter(|root| {
            nif.blocks
                .get(*root)
                .is_some_and(|block| matches!(block.type_name.as_str(), "NiNode" | "BSFadeNode"))
        })
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        return Err(LegacyCreatureRagdollError::Install {
            reason: format!(
                "visual skeleton has no unique scene root for a CharacterController node"
            ),
        });
    };
    let root = *root;
    let controller = nif.add_block("NiNode", None);
    nif.blocks[controller].set_field("Name", NifValue::String("CharacterController".to_string()));
    let children = nif.blocks[root]
        .get_field_mut("Children")
        .and_then(|value| match value {
            NifValue::Array(children) => Some(children),
            _ => None,
        })
        .ok_or_else(|| LegacyCreatureRagdollError::Install {
            reason: format!("visual skeleton root {root} has no Children array"),
        })?;
    children.push(NifValue::Ref(controller as i32));
    Ok(controller)
}

fn require_open_collision_slot(
    nif: &NifFile,
    target: usize,
) -> Result<(), LegacyCreatureRagdollError> {
    let occupied = nif.blocks[target]
        .get_field("Collision Object")
        .and_then(reference)
        .is_some_and(|reference| reference >= 0);
    if occupied {
        return Err(LegacyCreatureRagdollError::Install {
            reason: format!("visual skeleton node {target} already has collision"),
        });
    }
    Ok(())
}

fn install_np_collision_object(
    nif: &mut NifFile,
    target: usize,
    system_id: usize,
    body_id: usize,
) -> Result<(), LegacyCreatureRagdollError> {
    let mut collision_fields = IndexMap::new();
    collision_fields.insert("Target".to_string(), NifValue::Ref(target as i32));
    collision_fields.insert("Flags".to_string(), NifValue::UInt(0x80));
    collision_fields.insert("Data".to_string(), NifValue::Ref(system_id as i32));
    collision_fields.insert("Body ID".to_string(), NifValue::UInt(body_id as u64));
    let collision_id = nif.add_block("bhkNPCollisionObject", Some(collision_fields));
    let target_node =
        nif.blocks
            .get_mut(target)
            .ok_or_else(|| LegacyCreatureRagdollError::Install {
                reason: format!("visual skeleton collision target {target} disappeared"),
            })?;
    target_node.set_field("Collision Object", NifValue::Ref(collision_id as i32));
    Ok(())
}

fn node_parents(
    nif: &NifFile,
    node_ids: &HashSet<usize>,
) -> Result<HashMap<usize, usize>, LegacyCreatureRagdollError> {
    let mut parents = HashMap::new();
    for parent in node_ids {
        for child in ref_array(nif.blocks[*parent].get_field("Children")) {
            if !node_ids.contains(&child) {
                continue;
            }
            if parents.insert(child, *parent).is_some() {
                return Err(LegacyCreatureRagdollError::MultipleNodeParents { node: child });
            }
        }
    }
    for node in node_ids {
        let mut visited = HashSet::new();
        let mut current = *node;
        while let Some(parent) = parents.get(&current).copied() {
            if !visited.insert(current) {
                return Err(LegacyCreatureRagdollError::NodeCycle { node: current });
            }
            current = parent;
        }
    }
    Ok(parents)
}

fn body_targets(
    nif: &NifFile,
    node_ids: &HashSet<usize>,
) -> Result<HashMap<usize, usize>, LegacyCreatureRagdollError> {
    let mut result = HashMap::new();
    for collision_id in nif.find_blocks("bhkNiCollisionObject") {
        let collision = &nif.blocks[collision_id];
        let target = required_ref(nif, collision, "Target")?;
        let body = required_ref(nif, collision, "Body")?;
        if !node_ids.contains(&target) {
            return Err(LegacyCreatureRagdollError::InvalidReference {
                block: collision_id,
                field: "Target".to_string(),
                target: target as i32,
            });
        }
        if !matches!(
            nif.blocks[body].type_name.as_str(),
            "bhkRigidBody" | "bhkRigidBodyT"
        ) {
            continue;
        }
        if let Some(previous) = result.insert(body, target)
            && previous != target
        {
            return Err(LegacyCreatureRagdollError::DuplicateBodyTarget { body });
        }
    }
    Ok(result)
}

fn source_constraints(
    nif: &NifFile,
    body_targets: &HashMap<usize, usize>,
) -> Result<Vec<SourceConstraint>, LegacyCreatureRagdollError> {
    let mut result = Vec::new();
    for constraint_id in nif.find_blocks("bhkConstraint") {
        let constraint = &nif.blocks[constraint_id];
        let info = required_struct(constraint, "Constraint Info")?;
        let body_a = struct_ref(info, "Entity A").ok_or_else(|| {
            LegacyCreatureRagdollError::MissingField {
                block: constraint_id,
                field: "Constraint Info.Entity A".to_string(),
            }
        })?;
        let body_b = struct_ref(info, "Entity B").ok_or_else(|| {
            LegacyCreatureRagdollError::MissingField {
                block: constraint_id,
                field: "Constraint Info.Entity B".to_string(),
            }
        })?;
        if !body_targets.contains_key(&body_a) || !body_targets.contains_key(&body_b) {
            continue;
        }
        let kind = match constraint.type_name.as_str() {
            "bhkLimitedHingeConstraint" => parse_limited_hinge(constraint)?,
            "bhkHingeConstraint" => parse_hinge(constraint)?,
            "bhkPrismaticConstraint" => parse_prismatic(constraint)?,
            "bhkBreakableConstraint" => parse_breakable(constraint)?,
            "bhkRagdollConstraint" => parse_ragdoll_constraint(constraint)?,
            unsupported => {
                return Err(LegacyCreatureRagdollError::UnsupportedConstraint {
                    constraint: constraint_id,
                    kind: unsupported.to_string(),
                });
            }
        };
        result.push(SourceConstraint {
            block_id: constraint_id,
            body_a,
            body_b,
            kind,
        });
    }
    Ok(result)
}

fn connected_components(
    bodies: &HashSet<usize>,
    constraints: &[SourceConstraint],
) -> Vec<HashSet<usize>> {
    let mut adjacency = HashMap::<usize, Vec<usize>>::new();
    for constraint in constraints {
        adjacency
            .entry(constraint.body_a)
            .or_default()
            .push(constraint.body_b);
        adjacency
            .entry(constraint.body_b)
            .or_default()
            .push(constraint.body_a);
    }
    let mut remaining = bodies.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.iter().next().copied() {
        let mut component = HashSet::new();
        let mut stack = vec![start];
        while let Some(body) = stack.pop() {
            if !component.insert(body) {
                continue;
            }
            remaining.remove(&body);
            stack.extend(adjacency.get(&body).into_iter().flatten().copied());
        }
        components.push(component);
    }
    components
}

fn body_tree(
    bodies: &HashSet<usize>,
    constraints: &[SourceConstraint],
) -> Result<(usize, HashMap<usize, usize>, Vec<usize>), LegacyCreatureRagdollError> {
    if constraints.len() + 1 != bodies.len() {
        return Err(LegacyCreatureRagdollError::NonTree {
            reason: format!(
                "{} bodies require {} edges, found {}",
                bodies.len(),
                bodies.len().saturating_sub(1),
                constraints.len()
            ),
        });
    }
    let mut parents = HashMap::new();
    let mut children = HashMap::<usize, Vec<usize>>::new();
    for constraint in constraints {
        if parents
            .insert(constraint.body_a, constraint.body_b)
            .is_some()
        {
            return Err(LegacyCreatureRagdollError::NonTree {
                reason: format!("body {} has multiple parents", constraint.body_a),
            });
        }
        children
            .entry(constraint.body_b)
            .or_default()
            .push(constraint.body_a);
    }
    let roots = bodies
        .iter()
        .filter(|body| !parents.contains_key(body))
        .copied()
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err(LegacyCreatureRagdollError::NonTree {
            reason: format!("expected one root, found {}", roots.len()),
        });
    }
    for values in children.values_mut() {
        values.sort_unstable();
    }
    let mut order = Vec::with_capacity(bodies.len());
    let mut stack = vec![roots[0]];
    while let Some(body) = stack.pop() {
        if order.contains(&body) {
            return Err(LegacyCreatureRagdollError::NonTree {
                reason: format!("cycle reaches body {body}"),
            });
        }
        order.push(body);
        if let Some(body_children) = children.get(&body) {
            stack.extend(body_children.iter().rev().copied());
        }
    }
    if order.len() != bodies.len() {
        return Err(LegacyCreatureRagdollError::NonTree {
            reason: "directed closure is disconnected".to_string(),
        });
    }
    Ok((roots[0], parents, order))
}

fn allocate_filter_ids(
    body_order: &[usize],
    parent_of_body: &HashMap<usize, usize>,
) -> Result<HashMap<usize, u32>, LegacyCreatureRagdollError> {
    let parent_bodies = parent_of_body.values().copied().collect::<HashSet<_>>();
    let required_ids = parent_bodies.len() + 1;
    if required_ids > 32 {
        return Err(LegacyCreatureRagdollError::CollisionFilterIdExhausted {
            bodies: body_order.len(),
            required_ids,
        });
    }
    let mut next_parent_id = 1u32;
    Ok(body_order
        .iter()
        .map(|body| {
            let id = if parent_bodies.contains(body) {
                let id = next_parent_id;
                next_parent_id += 1;
                id
            } else {
                0
            };
            (*body, id)
        })
        .collect())
}

fn animation_root(start: usize, nif: &NifFile, parents: &HashMap<usize, usize>) -> usize {
    let mut root = start;
    while let Some(parent) = parents.get(&root).copied() {
        if nif.blocks[parent].type_name == "BSFadeNode" {
            break;
        }
        root = parent;
    }
    root
}

fn animation_skeleton(
    nif: &NifFile,
    root: usize,
    node_ids: &HashSet<usize>,
    body_targets: &HashMap<usize, usize>,
    body_ids: &HashSet<usize>,
) -> Result<(RigSkeletonIr, HashMap<usize, Transform>), LegacyCreatureRagdollError> {
    let mut bones = Vec::new();
    let mut indices = HashMap::new();
    let mut world = HashMap::new();
    let mut names = HashSet::new();
    let mut stack = vec![(root, None, Transform::identity())];
    while let Some((node, parent, parent_world)) = stack.pop() {
        let block = &nif.blocks[node];
        let name = bone_name(block)?;
        if !names.insert(name.clone()) {
            return Err(LegacyCreatureRagdollError::InvalidBoneName { node });
        }
        let local = node_local_transform(block)?;
        let node_world = parent_world.combine(local);
        let index = bones.len();
        indices.insert(node, index);
        world.insert(node, node_world);
        bones.push(RigBoneIr {
            name,
            parent,
            reference_pose: local.qs(),
            lock_translation: false,
        });
        let children = ref_array(block.get_field("Children"))
            .into_iter()
            .filter(|child| node_ids.contains(child))
            .collect::<Vec<_>>();
        for child in children.into_iter().rev() {
            stack.push((child, Some(index), node_world));
        }
    }
    for body in body_ids {
        let target = body_targets[body];
        if !indices.contains_key(&target) {
            return Err(LegacyCreatureRagdollError::TargetOutsideAnimationRoot {
                node: target,
                root,
            });
        }
    }
    Ok((
        RigSkeletonIr {
            name: format!("{}_Animation", bone_name(&nif.blocks[root])?),
            bones,
        },
        world,
    ))
}

fn parse_body(
    nif: &NifFile,
    body_id: usize,
    ragdoll_bone: &str,
    name: &str,
    world: Transform,
    collision_filter_info: u32,
) -> Result<RigidBodyIr, LegacyCreatureRagdollError> {
    let body = &nif.blocks[body_id];
    let shape_id = required_ref(nif, body, "Shape")?;
    let shape = parse_shape(nif, body_id, shape_id)?;
    let info = required_struct(body, "Rigid Body Info:550_660")?;
    let mass = required_float(info, "Mass", body_id)?;
    let center = required_vec3(info, "Center", body_id)?;
    let inertia = required_matrix33(info, "Inertia Tensor", body_id)?;
    let largest_diagonal = inertia[0][0]
        .abs()
        .max(inertia[1][1].abs())
        .max(inertia[2][2].abs())
        .max(1.0);
    for (row, values) in inertia.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            if row != column && value.abs() > largest_diagonal * 1.0e-4 {
                return Err(LegacyCreatureRagdollError::OffDiagonalInertia {
                    body: body_id,
                    row,
                    column,
                });
            }
        }
    }
    let inertia_scale = LEGACY_BHK_TO_FO4_CHARACTER_SCALE.powi(2);
    Ok(RigidBodyIr {
        name: name.to_string(),
        ragdoll_bone: ragdoll_bone.to_string(),
        shape,
        world_from_body: world.qs(),
        mass_properties: MassPropertiesIr {
            mass,
            center_of_mass: scale3(center, LEGACY_BHK_TO_FO4_CHARACTER_SCALE),
            inertia_diagonal: [
                inertia[0][0] * inertia_scale,
                inertia[1][1] * inertia_scale,
                inertia[2][2] * inertia_scale,
            ],
        },
        friction: required_float(info, "Friction", body_id)?,
        restitution: required_float(info, "Restitution", body_id)?,
        linear_damping: required_float(info, "Linear Damping", body_id)?,
        angular_damping: required_float(info, "Angular Damping", body_id)?,
        collision_filter_info,
    })
}

fn parse_shape(
    nif: &NifFile,
    body_id: usize,
    shape_id: usize,
) -> Result<RagdollShapeIr, LegacyCreatureRagdollError> {
    parse_shape_recursive(nif, body_id, shape_id, &mut HashSet::new())
}

fn parse_shape_recursive(
    nif: &NifFile,
    body_id: usize,
    shape_id: usize,
    visited: &mut HashSet<usize>,
) -> Result<RagdollShapeIr, LegacyCreatureRagdollError> {
    if !visited.insert(shape_id) {
        return Err(LegacyCreatureRagdollError::UnsupportedCompoundShape {
            body: body_id,
            shape: "cyclic compound shape".to_string(),
        });
    }
    let shape = &nif.blocks[shape_id];
    let factor = LEGACY_BHK_TO_FO4_CHARACTER_SCALE;
    let parsed = match shape.type_name.as_str() {
        "bhkCapsuleShape" => {
            let vertex_a = required_block_vec3(shape, "First Point")?;
            let vertex_b = required_block_vec3(shape, "Second Point")?;
            let radius_a = required_block_float(shape, "Radius 1")?;
            let radius_b = required_block_float(shape, "Radius 2")?;
            if (radius_a - radius_b).abs() > radius_a.abs().max(radius_b.abs()).max(1.0) * 1.0e-4 {
                return Err(LegacyCreatureRagdollError::UnsupportedShape {
                    body: body_id,
                    shape: "tapered bhkCapsuleShape".to_string(),
                });
            }
            Ok(RagdollShapeIr::Capsule {
                vertex_a: scale3(vertex_a, factor),
                vertex_b: scale3(vertex_b, factor),
                radius: 0.5 * (radius_a + radius_b) * factor,
            })
        }
        "bhkSphereShape" => Ok(RagdollShapeIr::Sphere {
            center: [0.0; 3],
            radius: required_block_float(shape, "Radius")? * factor,
        }),
        "bhkBoxShape" => Ok(RagdollShapeIr::Box {
            half_extents: scale3(required_block_vec3(shape, "Dimensions")?, factor),
            convex_radius: 0.0,
        }),
        "bhkConvexVerticesShape" => {
            let vertices = vec4_array(shape.get_field("Vertices"))
                .ok_or_else(|| LegacyCreatureRagdollError::MissingField {
                    block: shape_id,
                    field: "Vertices".to_string(),
                })?
                .into_iter()
                .map(|value| scale3([value[0], value[1], value[2]], factor))
                .collect();
            let planes = vec4_array(shape.get_field("Normals"))
                .ok_or_else(|| LegacyCreatureRagdollError::MissingField {
                    block: shape_id,
                    field: "Normals".to_string(),
                })?
                .into_iter()
                .map(|value| [value[0], value[1], value[2], value[3] * factor])
                .collect();
            Ok(RagdollShapeIr::ConvexHull(ConvexHullIr {
                vertices,
                planes,
                convex_radius: optional_block_float(shape, "Radius").unwrap_or(0.0) * factor,
            }))
        }
        "bhkListShape" | "bhkConvexListShape" => {
            let child_ids = ref_array(shape.get_field("Sub Shapes"));
            if child_ids.is_empty() {
                return Err(LegacyCreatureRagdollError::UnsupportedCompoundShape {
                    body: body_id,
                    shape: format!("empty {}", shape.type_name),
                });
            }
            let filters = match shape.get_field("Filters") {
                Some(NifValue::Array(values)) => values,
                _ => {
                    return Err(LegacyCreatureRagdollError::MissingField {
                        block: shape_id,
                        field: "Filters".to_string(),
                    });
                }
            };
            if filters.len() != child_ids.len() || !filters.iter().all(zero_collision_filter) {
                return Err(LegacyCreatureRagdollError::UnsupportedCompoundShape {
                    body: body_id,
                    shape: format!("{} with nonzero child filters", shape.type_name),
                });
            }
            let mut children = Vec::with_capacity(child_ids.len());
            for child_id in child_ids {
                if child_id >= nif.blocks.len() {
                    return Err(LegacyCreatureRagdollError::InvalidReference {
                        block: shape_id,
                        field: "Sub Shapes".to_string(),
                        target: child_id as i32,
                    });
                }
                children.push(parse_shape_recursive(nif, body_id, child_id, visited)?);
            }
            Ok(RagdollShapeIr::Compound { children })
        }
        unsupported => Err(LegacyCreatureRagdollError::UnsupportedShape {
            body: body_id,
            shape: unsupported.to_string(),
        }),
    };
    visited.remove(&shape_id);
    parsed
}

fn zero_collision_filter(value: &NifValue) -> bool {
    let NifValue::Struct(fields) = value else {
        return false;
    };
    ["Layer", "Flags", "Group"].iter().all(|name| {
        struct_get(fields, name)
            .map(NifValue::as_i64)
            .is_some_and(|value| value == 0)
    })
}

fn parse_limited_hinge(block: &NifBlock) -> Result<ConstraintKindIr, LegacyCreatureRagdollError> {
    let fields = required_struct(block, "Constraint")?;
    Ok(ConstraintKindIr::LimitedHinge(parse_limited_hinge_fields(
        block.block_id,
        fields,
    )?))
}

fn parse_limited_hinge_fields(
    block_id: usize,
    fields: &IndexMap<String, NifValue>,
) -> Result<LimitedHingeIr, LegacyCreatureRagdollError> {
    Ok(LimitedHingeIr {
        frame_a: constraint_frame(
            block_id,
            "A",
            required_vec3(fields, "Axis A", block_id)?,
            required_vec3(fields, "Perp Axis In A1", block_id)?,
            required_vec3(fields, "Perp Axis In A2", block_id)?,
            required_vec3(fields, "Pivot A", block_id)?,
        )?,
        frame_b: constraint_frame(
            block_id,
            "B",
            required_vec3(fields, "Axis B", block_id)?,
            required_vec3(fields, "Perp Axis In B1", block_id)?,
            required_vec3(fields, "Perp Axis In B2", block_id)?,
            required_vec3(fields, "Pivot B", block_id)?,
        )?,
        min_angle: required_float(fields, "Min Angle", block_id)?,
        max_angle: required_float(fields, "Max Angle", block_id)?,
        max_friction_torque: required_float(fields, "Max Friction", block_id)?,
    })
}

fn parse_hinge(block: &NifBlock) -> Result<ConstraintKindIr, LegacyCreatureRagdollError> {
    let fields = required_struct(block, "Constraint")?;
    Ok(ConstraintKindIr::Hinge(HingeIr {
        frame_a: constraint_frame(
            block.block_id,
            "A",
            required_vec3(fields, "Axis A", block.block_id)?,
            required_vec3(fields, "Perp Axis In A1", block.block_id)?,
            required_vec3(fields, "Perp Axis In A2", block.block_id)?,
            required_vec3(fields, "Pivot A", block.block_id)?,
        )?,
        frame_b: constraint_frame(
            block.block_id,
            "B",
            required_vec3(fields, "Axis B", block.block_id)?,
            required_vec3(fields, "Perp Axis In B1", block.block_id)?,
            required_vec3(fields, "Perp Axis In B2", block.block_id)?,
            required_vec3(fields, "Pivot B", block.block_id)?,
        )?,
    }))
}

fn parse_prismatic(block: &NifBlock) -> Result<ConstraintKindIr, LegacyCreatureRagdollError> {
    let fields = required_struct(block, "Constraint")?;
    Ok(ConstraintKindIr::Prismatic(PrismaticIr {
        frame_a: constraint_frame(
            block.block_id,
            "A",
            required_vec3(fields, "Sliding A", block.block_id)?,
            required_vec3(fields, "Rotation A", block.block_id)?,
            required_vec3(fields, "Plane A", block.block_id)?,
            required_vec3(fields, "Pivot A", block.block_id)?,
        )?,
        frame_b: constraint_frame(
            block.block_id,
            "B",
            required_vec3(fields, "Sliding B", block.block_id)?,
            required_vec3(fields, "Rotation B", block.block_id)?,
            required_vec3(fields, "Plane B", block.block_id)?,
            required_vec3(fields, "Pivot B", block.block_id)?,
        )?,
        min_distance: required_float(fields, "Min Distance", block.block_id)?
            * LEGACY_BHK_TO_FO4_CHARACTER_SCALE,
        max_distance: required_float(fields, "Max Distance", block.block_id)?
            * LEGACY_BHK_TO_FO4_CHARACTER_SCALE,
        max_friction_force: required_float(fields, "Friction", block.block_id)?,
    }))
}

fn parse_breakable(block: &NifBlock) -> Result<ConstraintKindIr, LegacyCreatureRagdollError> {
    let fields = required_struct(block, "Constraint Data")?;
    let constraint_type = struct_get(fields, "Type").map(NifValue::as_i64);
    let remove_when_broken = block.get_field("Remove When Broken").map(NifValue::as_i64);
    if constraint_type != Some(2) || remove_when_broken != Some(0) {
        return Err(LegacyCreatureRagdollError::UnsupportedConstraint {
            constraint: block.block_id,
            kind: "bhkBreakableConstraint semantics".to_string(),
        });
    }
    let limited_hinge = match struct_get(fields, "Limited Hinge") {
        Some(NifValue::Struct(fields)) => fields,
        _ => {
            return Err(LegacyCreatureRagdollError::MissingField {
                block: block.block_id,
                field: "Constraint Data.Limited Hinge".to_string(),
            });
        }
    };
    Ok(ConstraintKindIr::Breakable {
        inner: Box::new(ConstraintKindIr::LimitedHinge(parse_limited_hinge_fields(
            block.block_id,
            limited_hinge,
        )?)),
        threshold: required_block_float(block, "Threshold")?,
    })
}

fn parse_ragdoll_constraint(
    block: &NifBlock,
) -> Result<ConstraintKindIr, LegacyCreatureRagdollError> {
    let fields = required_struct(block, "Constraint")?;
    Ok(ConstraintKindIr::Ragdoll(RagdollLimitsIr {
        frame_a: constraint_frame(
            block.block_id,
            "A",
            required_vec3(fields, "Twist A", block.block_id)?,
            required_vec3(fields, "Plane A", block.block_id)?,
            required_vec3(fields, "Motor A", block.block_id)?,
            required_vec3(fields, "Pivot A", block.block_id)?,
        )?,
        frame_b: constraint_frame(
            block.block_id,
            "B",
            required_vec3(fields, "Twist B", block.block_id)?,
            required_vec3(fields, "Plane B", block.block_id)?,
            required_vec3(fields, "Motor B", block.block_id)?,
            required_vec3(fields, "Pivot B", block.block_id)?,
        )?,
        cone_limit: required_float(fields, "Cone Max Angle", block.block_id)?,
        plane_min: required_float(fields, "Plane Min Angle", block.block_id)?,
        plane_max: required_float(fields, "Plane Max Angle", block.block_id)?,
        twist_min: required_float(fields, "Twist Min Angle", block.block_id)?,
        twist_max: required_float(fields, "Twist Max Angle", block.block_id)?,
        max_friction_torque: required_float(fields, "Max Friction", block.block_id)?,
    }))
}

fn constraint_frame(
    constraint: usize,
    frame: &str,
    axis: [f32; 3],
    perpendicular: [f32; 3],
    source_third: [f32; 3],
    pivot: [f32; 3],
) -> Result<QsTransformIr, LegacyCreatureRagdollError> {
    let x =
        normalize3(axis).ok_or_else(|| LegacyCreatureRagdollError::DegenerateConstraintFrame {
            constraint,
            frame: frame.to_string(),
        })?;
    let projected = sub(perpendicular, scale3(x, dot(x, perpendicular)));
    let mut y = normalize3(projected).ok_or_else(|| {
        LegacyCreatureRagdollError::DegenerateConstraintFrame {
            constraint,
            frame: frame.to_string(),
        }
    })?;
    let mut z = normalize3(cross(x, y)).ok_or_else(|| {
        LegacyCreatureRagdollError::DegenerateConstraintFrame {
            constraint,
            frame: frame.to_string(),
        }
    })?;
    if dot(z, source_third) < 0.0 {
        y = scale3(y, -1.0);
        z = scale3(z, -1.0);
    }
    let matrix = [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]];
    Ok(QsTransformIr {
        translation: scale3(pivot, LEGACY_BHK_TO_FO4_CHARACTER_SCALE),
        rotation: quaternion_from_matrix(matrix),
        scale: [1.0; 3],
    })
}

fn node_local_transform(block: &NifBlock) -> Result<Transform, LegacyCreatureRagdollError> {
    let translation = block
        .get_field("Translation")
        .and_then(vec3)
        .unwrap_or([0.0; 3]);
    let matrix = block.get_field("Rotation").and_then(matrix33).unwrap_or([
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ]);
    let matrix = transpose(matrix);
    let scale = block.get_field("Scale").and_then(float).unwrap_or(1.0);
    finite_transform(
        block.block_id,
        translation,
        quaternion_from_matrix(matrix),
        scale,
    )
}

fn body_local_transform(block: &NifBlock) -> Result<Transform, LegacyCreatureRagdollError> {
    let info = required_struct(block, "Rigid Body Info:550_660")?;
    let translation = scale3(
        required_vec3(info, "Translation", block.block_id)?,
        LEGACY_BHK_TO_FO4_CHARACTER_SCALE,
    );
    let rotation = struct_get(info, "Rotation")
        .and_then(quaternion)
        .and_then(normalize_quaternion)
        .ok_or_else(|| LegacyCreatureRagdollError::InvalidValue {
            path: format!(
                "blocks[{}].Rigid Body Info:550_660.Rotation",
                block.block_id
            ),
        })?;
    finite_transform(block.block_id, translation, rotation, 1.0)
}

fn finite_transform(
    block: usize,
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: f32,
) -> Result<Transform, LegacyCreatureRagdollError> {
    if translation
        .iter()
        .chain(rotation.iter())
        .any(|value| !value.is_finite())
        || !scale.is_finite()
        || scale <= 0.0
    {
        return Err(LegacyCreatureRagdollError::InvalidValue {
            path: format!("blocks[{block}].transform"),
        });
    }
    Ok(Transform {
        translation,
        rotation,
        scale,
    })
}

fn hkp_group_filter_info(body_id: u32, parent_id: u32) -> u32 {
    (body_id << 5) | (parent_id << 10) | (1 << 16)
}

fn source_name(nif: &NifFile) -> String {
    nif.path
        .as_deref()
        .and_then(|path| path.parent())
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("LegacyCreature")
        .to_string()
}

fn bone_name(block: &NifBlock) -> Result<String, LegacyCreatureRagdollError> {
    match block.get_field("Name") {
        Some(NifValue::String(name)) if !name.is_empty() => Ok(name.clone()),
        _ => Err(LegacyCreatureRagdollError::InvalidBoneName {
            node: block.block_id,
        }),
    }
}

fn required_ref(
    nif: &NifFile,
    block: &NifBlock,
    field: &str,
) -> Result<usize, LegacyCreatureRagdollError> {
    let raw = block.get_field(field).and_then(reference).ok_or_else(|| {
        LegacyCreatureRagdollError::MissingField {
            block: block.block_id,
            field: field.to_string(),
        }
    })?;
    if raw < 0 || raw as usize >= nif.blocks.len() {
        return Err(LegacyCreatureRagdollError::InvalidReference {
            block: block.block_id,
            field: field.to_string(),
            target: raw,
        });
    }
    Ok(raw as usize)
}

fn required_struct<'a>(
    block: &'a NifBlock,
    field: &str,
) -> Result<&'a IndexMap<String, NifValue>, LegacyCreatureRagdollError> {
    match block.get_field(field) {
        Some(NifValue::Struct(fields)) => Ok(fields),
        _ => Err(LegacyCreatureRagdollError::MissingField {
            block: block.block_id,
            field: field.to_string(),
        }),
    }
}

fn required_float(
    fields: &IndexMap<String, NifValue>,
    field: &str,
    block: usize,
) -> Result<f32, LegacyCreatureRagdollError> {
    struct_get(fields, field)
        .and_then(float)
        .filter(|value| value.is_finite())
        .ok_or_else(|| LegacyCreatureRagdollError::InvalidValue {
            path: format!("blocks[{block}].{field}"),
        })
}

fn required_vec3(
    fields: &IndexMap<String, NifValue>,
    field: &str,
    block: usize,
) -> Result<[f32; 3], LegacyCreatureRagdollError> {
    struct_get(fields, field)
        .and_then(vec3)
        .filter(|value| value.iter().all(|component| component.is_finite()))
        .ok_or_else(|| LegacyCreatureRagdollError::InvalidValue {
            path: format!("blocks[{block}].{field}"),
        })
}

fn required_matrix33(
    fields: &IndexMap<String, NifValue>,
    field: &str,
    block: usize,
) -> Result<[[f32; 3]; 3], LegacyCreatureRagdollError> {
    struct_get(fields, field)
        .and_then(matrix33)
        .filter(|value| {
            value
                .iter()
                .flatten()
                .all(|component| component.is_finite())
        })
        .ok_or_else(|| LegacyCreatureRagdollError::InvalidValue {
            path: format!("blocks[{block}].{field}"),
        })
}

fn required_block_float(block: &NifBlock, field: &str) -> Result<f32, LegacyCreatureRagdollError> {
    optional_block_float(block, field).ok_or_else(|| LegacyCreatureRagdollError::InvalidValue {
        path: format!("blocks[{}].{field}", block.block_id),
    })
}

fn optional_block_float(block: &NifBlock, field: &str) -> Option<f32> {
    block
        .get_field(field)
        .and_then(float)
        .filter(|value| value.is_finite())
}

fn required_block_vec3(
    block: &NifBlock,
    field: &str,
) -> Result<[f32; 3], LegacyCreatureRagdollError> {
    block
        .get_field(field)
        .and_then(vec3)
        .filter(|value| value.iter().all(|component| component.is_finite()))
        .ok_or_else(|| LegacyCreatureRagdollError::InvalidValue {
            path: format!("blocks[{}].{field}", block.block_id),
        })
}

fn struct_get<'a>(fields: &'a IndexMap<String, NifValue>, name: &str) -> Option<&'a NifValue> {
    fields.get(name).or_else(|| {
        fields
            .iter()
            .find(|(key, _)| key.split(':').next() == Some(name))
            .map(|(_, value)| value)
    })
}

fn struct_ref(fields: &IndexMap<String, NifValue>, name: &str) -> Option<usize> {
    let value = struct_get(fields, name).and_then(reference)?;
    (value >= 0).then_some(value as usize)
}

fn ref_array(value: Option<&NifValue>) -> Vec<usize> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(reference)
            .filter(|value| *value >= 0)
            .map(|value| value as usize)
            .collect(),
        _ => Vec::new(),
    }
}

fn reference(value: &NifValue) -> Option<i32> {
    match value {
        NifValue::Ref(value) => Some(*value),
        NifValue::Int(value) => i32::try_from(*value).ok(),
        NifValue::UInt(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

fn float(value: &NifValue) -> Option<f32> {
    match value {
        NifValue::Float(value) => Some(*value as f32),
        NifValue::Int(value) => Some(*value as f32),
        NifValue::UInt(value) => Some(*value as f32),
        _ => None,
    }
}

fn vec3(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(value) => Some(*value),
        NifValue::Vec4(value) | NifValue::Color4(value) => Some([value[0], value[1], value[2]]),
        NifValue::Struct(fields) => Some([
            float(struct_get(fields, "x")?)?,
            float(struct_get(fields, "y")?)?,
            float(struct_get(fields, "z")?)?,
        ]),
        _ => None,
    }
}

fn quaternion(value: &NifValue) -> Option<[f32; 4]> {
    match value {
        NifValue::Quaternion(value) | NifValue::Vec4(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            float(struct_get(fields, "x")?)?,
            float(struct_get(fields, "y")?)?,
            float(struct_get(fields, "z")?)?,
            float(struct_get(fields, "w")?)?,
        ]),
        _ => None,
    }
}

fn matrix33(value: &NifValue) -> Option<[[f32; 3]; 3]> {
    match value {
        NifValue::Matrix33(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            [
                float(struct_get(fields, "m11")?)?,
                float(struct_get(fields, "m21")?)?,
                float(struct_get(fields, "m31")?)?,
            ],
            [
                float(struct_get(fields, "m12")?)?,
                float(struct_get(fields, "m22")?)?,
                float(struct_get(fields, "m32")?)?,
            ],
            [
                float(struct_get(fields, "m13")?)?,
                float(struct_get(fields, "m23")?)?,
                float(struct_get(fields, "m33")?)?,
            ],
        ]),
        _ => None,
    }
}

fn vec4_array(value: Option<&NifValue>) -> Option<Vec<[f32; 4]>> {
    let NifValue::Array(values) = value? else {
        return None;
    };
    values.iter().map(vec4).collect()
}

fn vec4(value: &NifValue) -> Option<[f32; 4]> {
    match value {
        NifValue::Vec4(value) | NifValue::Color4(value) | NifValue::Quaternion(value) => {
            Some(*value)
        }
        NifValue::Struct(fields) => Some([
            float(struct_get(fields, "x")?)?,
            float(struct_get(fields, "y")?)?,
            float(struct_get(fields, "z")?)?,
            float(struct_get(fields, "w")?)?,
        ]),
        _ => None,
    }
}

fn transpose(matrix: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
        [matrix[0][0], matrix[1][0], matrix[2][0]],
        [matrix[0][1], matrix[1][1], matrix[2][1]],
        [matrix[0][2], matrix[1][2], matrix[2][2]],
    ]
}

fn quaternion_from_matrix(matrix: [[f32; 3]; 3]) -> [f32; 4] {
    let trace = matrix[0][0] + matrix[1][1] + matrix[2][2];
    let result = if trace > 0.0 {
        let scale = (trace + 1.0).sqrt() * 2.0;
        [
            (matrix[2][1] - matrix[1][2]) / scale,
            (matrix[0][2] - matrix[2][0]) / scale,
            (matrix[1][0] - matrix[0][1]) / scale,
            0.25 * scale,
        ]
    } else if matrix[0][0] > matrix[1][1] && matrix[0][0] > matrix[2][2] {
        let scale = (1.0 + matrix[0][0] - matrix[1][1] - matrix[2][2]).sqrt() * 2.0;
        [
            0.25 * scale,
            (matrix[0][1] + matrix[1][0]) / scale,
            (matrix[0][2] + matrix[2][0]) / scale,
            (matrix[2][1] - matrix[1][2]) / scale,
        ]
    } else if matrix[1][1] > matrix[2][2] {
        let scale = (1.0 + matrix[1][1] - matrix[0][0] - matrix[2][2]).sqrt() * 2.0;
        [
            (matrix[0][1] + matrix[1][0]) / scale,
            0.25 * scale,
            (matrix[1][2] + matrix[2][1]) / scale,
            (matrix[0][2] - matrix[2][0]) / scale,
        ]
    } else {
        let scale = (1.0 + matrix[2][2] - matrix[0][0] - matrix[1][1]).sqrt() * 2.0;
        [
            (matrix[0][2] + matrix[2][0]) / scale,
            (matrix[1][2] + matrix[2][1]) / scale,
            0.25 * scale,
            (matrix[1][0] - matrix[0][1]) / scale,
        ]
    };
    normalize_quaternion(result).unwrap_or([0.0, 0.0, 0.0, 1.0])
}

fn normalize_quaternion(value: [f32; 4]) -> Option<[f32; 4]> {
    let length = value
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt();
    (length.is_finite() && length > f32::EPSILON).then(|| value.map(|component| component / length))
}

fn quat_mul(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
    [
        left[3] * right[0] + left[0] * right[3] + left[1] * right[2] - left[2] * right[1],
        left[3] * right[1] - left[0] * right[2] + left[1] * right[3] + left[2] * right[0],
        left[3] * right[2] + left[0] * right[1] - left[1] * right[0] + left[2] * right[3],
        left[3] * right[3] - left[0] * right[0] - left[1] * right[1] - left[2] * right[2],
    ]
}

fn quat_conjugate(value: [f32; 4]) -> [f32; 4] {
    [-value[0], -value[1], -value[2], value[3]]
}

fn rotate(quaternion: [f32; 4], point: [f32; 3]) -> [f32; 3] {
    let vector = [quaternion[0], quaternion[1], quaternion[2]];
    let uv = cross(vector, point);
    let uuv = cross(vector, uv);
    add(
        point,
        add(scale3(uv, 2.0 * quaternion[3]), scale3(uuv, 2.0)),
    )
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn sub(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale3(value: [f32; 3], scale: f32) -> [f32; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(value, value).sqrt();
    (length.is_finite() && length > f32::EPSILON).then(|| scale3(value, length.recip()))
}
