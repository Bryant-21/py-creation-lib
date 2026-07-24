use super::capsule::{SourceCapsuleShape, build_fo4_source_capsule_collision};
use super::compound::{CompoundChild, CompoundChildKind, build_fo4_compound_collision};
use super::compressed_mesh::{
    BuildOptions, MaterialEntry, RawCompressedMeshData, build_compressed_mesh_collision,
    build_compressed_mesh_collision_from_raw,
};
use super::constraints::GraftedConstraints;
use super::convex::{SourceConvexShape, build_fo4_source_convex_collision};
use super::polytope::{
    SourcePolytopeShape, build_fo4_polytope_collision, build_fo4_source_polytope_collision,
};
use super::sphere::build_fo4_sphere_collision;
use crate::animation::pose::quat_rotate;
use crate::error::{HavokError, HavokResult};
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

/// Sentinel used by Havok 2014 for "no motion linked"; matches HK_INVALID_OBJECT_INDEX (0x7FFFFFFF).
const MOTION_ID_INVALID: i32 = 0x7FFFFFFF;
/// `hknpBodyCinfo.flags` bit marking a body the solver simulates dynamically
/// (vanilla FO4 loose clutter = 128; static/keyframed bodies = 0).
const BODY_FLAGS_DYNAMIC: i64 = 128;
/// Sentinel "very large" f32 used by vanilla as the no-clamp value for
/// maxLinearAccelerationDistancePerStep / maxRotationToPreventTunneling.
/// Matches the value emitted by `convert::fo76::synthesize_motion_cinfos`.
const FLT_CAP: f32 = 1.844_672_6e19;
/// `collisionFilterInfo` bit marking a body as part of an articulated system —
/// bits 8..14 then carry the part number and the engine suppresses contacts
/// between parts of the same system (vanilla TrapCanChimes01: 0x800F,
/// 0x810A..0x850A).
const RAGDOLL_PART_FILTER_FLAG: u32 = 0x8000;

/// How an individual body in a multi-body packfile behaves at runtime.
///
/// Standalone `Static` compressed-mesh bodies can use `motionId = HK_INVALID`,
/// matching vanilla static set-dressing pieces.
///
/// `Keyframed` bodies (ANIMSTATIC) need a populated `motionCinfos` entry so
/// the broadphase can resolve a motion frame during sweep queries. Without
/// this, workshop placement / projectile casts crash with a null deref in
/// `hknpBSWorld` (FO4 1.10.x +0x13E82D0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BodyMotionType {
    #[default]
    Static,
    Keyframed,
}

/// Per-body metadata layered on top of the shared [`BuildOptions`].
///
/// `layer` is the FO4 collision layer byte (1=STATIC, 2=ANIMSTATIC, 4=CLUTTER, …)
/// written into `bodyCinfo.collisionFilterInfo`.
///
/// `position` / `orientation` are the body's world transform in Havok units
/// (NIF translation / havok_scale; rotation as XYZW quaternion). Default is
/// origin + identity. Keyframed bodies replicate these into the matching
/// motionCinfo's centerOfMassWorld + orientation.
#[derive(Debug, Clone, Copy)]
pub struct BodyMeta {
    pub layer: u8,
    /// Full source `hknpBodyCinfo.collisionFilterInfo` (layer byte + group/system
    /// high-bytes). `Some` only for constrained assemblies, where the per-body
    /// group bits (`0x81xx`, `0x82xx`, …) are load-bearing — they keep the
    /// articulated bodies from self-colliding. Written verbatim when the part
    /// bit (0x8000) is present; a constrained body without it gets a vanilla
    /// part filter synthesized (`0x8000 | body_index<<8 | layer`) since FO76
    /// sources may rely on runtime pair filtering FO4 doesn't do. Otherwise the
    /// filter is just `layer`.
    pub collision_filter_info: Option<u32>,
    /// Source `hknpBodyCinfo.flags` for non-dynamic bodies. FO76 trigger/bumper
    /// bodies can carry flags like 16 while still using static motion.
    pub body_flags: Option<i64>,
    /// Source `hknpMaterial.flags`, including `ENABLE_TRIGGER_MODIFIER`.
    pub material_flags: Option<i64>,
    /// Source `hknpMaterial.triggerType` for trigger-volume behavior.
    pub material_trigger_type: Option<u8>,
    pub position: [f32; 4],
    pub orientation: [f32; 4],
    pub motion_type: BodyMotionType,
    /// Explicit source `hknpBodyCinfo.mass`, when present. FO76 NIF collision
    /// bodies carry this even when they omit FO4-style `motionCinfos`.
    pub body_mass: Option<f32>,
    /// The source body's true mass distribution (FO76 `hknpRefMassDistribution`),
    /// when present. Carried into the shape mass-properties block + the dynamic
    /// motion cinfo instead of the AABB box approximation. `None` for statics
    /// and any body whose distribution couldn't be decoded.
    pub mass_distribution: Option<super::mass_properties::SourceMassDistribution>,
}

impl Default for BodyMeta {
    fn default() -> Self {
        Self {
            layer: 1,
            collision_filter_info: None,
            body_flags: None,
            material_flags: None,
            material_trigger_type: None,
            position: [0.0, 0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            motion_type: BodyMotionType::Static,
            body_mass: None,
            mass_distribution: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MultiBodyShape {
    Polytope {
        vertices: Vec<[f32; 3]>,
    },
    SourcePolytope {
        shape: SourcePolytopeShape,
    },
    CompressedMesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
    RawCompressedMesh {
        data: RawCompressedMeshData,
    },
    Compound {
        children: Vec<CompoundChild>,
    },
    /// Single-body sphere. Mirrors pynifly's `pack_sphere`
    /// (`refs/io_scene_nifly/pyn/bhk_autopack.py:1587`). Only supported as the
    /// sole body — multi-body compositions with a sphere are rejected. FO4
    /// set-dressing (poolballs, mines, fruit) uses this.
    Sphere {
        radius: f32,
        position: [f32; 3],
    },
    Capsule {
        shape: SourceCapsuleShape,
    },
    SourceConvex {
        shape: SourceConvexShape,
    },
}

pub fn build_fo4_multi_body_collision(
    bodies: &[MultiBodyShape],
    opts: &BuildOptions,
    material_crcs: Option<&[Option<u32>]>,
    body_metas: Option<&[BodyMeta]>,
) -> HavokResult<Vec<u8>> {
    build_fo4_multi_body_collision_with_constraints(bodies, opts, material_crcs, body_metas, None)
}

/// As [`build_fo4_multi_body_collision`], but grafts an articulated assembly's
/// constraint sub-graph (lifted by [`super::constraints::extract_grafted_constraints`])
/// back onto the rebuilt physics system. `constraints` carries body indices that
/// are already remapped to this call's body order.
pub fn build_fo4_multi_body_collision_with_constraints(
    bodies: &[MultiBodyShape],
    opts: &BuildOptions,
    material_crcs: Option<&[Option<u32>]>,
    body_metas: Option<&[BodyMeta]>,
    constraints: Option<&GraftedConstraints>,
) -> HavokResult<Vec<u8>> {
    if bodies.is_empty() {
        return Err(HavokError::InvalidInput(
            "at least one collision body is required".to_string(),
        ));
    }
    if let Some(metas) = body_metas {
        if metas.len() != bodies.len() {
            return Err(HavokError::InvalidInput(format!(
                "body_metas length {} does not match bodies length {}",
                metas.len(),
                bodies.len()
            )));
        }
    }

    // Single-body sphere short-circuits the HkxFile merge. The hknpSphereShape
    // body carries descriptor-unknown trailer bytes (0x30 marker + 0x4C trailer
    // float — see sphere.rs) that vanilla FO4 requires; a round-trip through
    // HkxFile::save would strip them. Multi-body sphere compositions are
    // rejected below since vanilla provides no precedent for them.
    if bodies.len() == 1 {
        if let MultiBodyShape::Sphere { radius, position } = bodies[0] {
            let body_opts = options_for_body(opts, &bodies[0], material_crcs, body_metas, 0);
            let meta = body_metas.and_then(|m| m.first()).copied();
            let center = meta
                .map(|meta| {
                    let local_center = quat_rotate(&meta.orientation, &position);
                    [
                        meta.position[0] + local_center[0],
                        meta.position[1] + local_center[1],
                        meta.position[2] + local_center[2],
                    ]
                })
                .unwrap_or(position);
            return build_fo4_sphere_collision(radius, center, &body_opts);
        }
    }

    // Pynifly's pack_mixed requires CM-before-polytope ordering and vanilla
    // Safe01-style FO4 assets always lay out CM body 0, polytope body 1. The
    // Python caller (`_install_fo4_multi_body_collision`) now sorts; enforce
    // here so direct callers can't slip a [Polytope, …, CompressedMesh, …]
    // composition past us — the runtime tolerance for that is unverified.
    let mut seen_polytope = false;
    let mut cm_count = 0usize;
    for (i, body) in bodies.iter().enumerate() {
        match body {
            MultiBodyShape::CompressedMesh { .. } | MultiBodyShape::RawCompressedMesh { .. }
                if seen_polytope =>
            {
                return Err(HavokError::InvalidInput(format!(
                    "body order violation: CompressedMesh body at index {i} follows a Polytope body; \
                     CM bodies must precede polytope bodies (vanilla FO4 convention; see pynifly pack_mixed)"
                )));
            }
            MultiBodyShape::CompressedMesh { .. } | MultiBodyShape::RawCompressedMesh { .. } => {
                cm_count += 1
            }
            MultiBodyShape::Polytope { .. } | MultiBodyShape::SourcePolytope { .. } => {
                seen_polytope = true
            }
            MultiBodyShape::Compound { .. } => {}
            // A capsule is its own convex body; it does not participate in the
            // CM-before-polytope ordering constraint.
            MultiBodyShape::Capsule { .. } | MultiBodyShape::SourceConvex { .. } => {}
            MultiBodyShape::Sphere { .. } => {
                return Err(HavokError::InvalidInput(format!(
                    "MultiBodyShape::Sphere at index {i} is only supported as a sole body. \
                     Vanilla FO4 has no Sphere+anything precedent (see pynifly pack_shapes \
                     in refs/io_scene_nifly/pyn/bhk_autopack.py:1538)."
                )));
            }
        }
    }
    // Vanilla FO4 CM assets use shared multi-CompressedMesh systems and mixed
    // DynamicCompound systems; keep those shared so each bhkNPCollisionObject
    // can address a body ID in one physics system. Pynifly does not expose
    // every one of these paths directly, but Fallout4.esm CM* meshes provide
    // the precedent.
    let default_meta = BodyMeta::default();
    let meta_at = |i: usize| -> BodyMeta { body_metas.map(|m| m[i]).unwrap_or(default_meta) };

    let mut objects: Vec<HkxObject> = Vec::new();
    let mut material_values: Vec<HkxValue> = Vec::with_capacity(bodies.len());
    let mut body_cinfo_values: Vec<HkxValue> = Vec::with_capacity(bodies.len());
    let mut referenced_values: Vec<HkxValue> = Vec::with_capacity(bodies.len());

    for (body_index, body) in bodies.iter().enumerate() {
        let body_opts = options_for_body(opts, body, material_crcs, body_metas, body_index);
        let blob = match body {
            MultiBodyShape::Polytope { vertices } => {
                build_fo4_polytope_collision(vertices, &body_opts)?
            }
            MultiBodyShape::SourcePolytope { shape } => {
                build_fo4_source_polytope_collision(shape, &body_opts)?
            }
            MultiBodyShape::CompressedMesh {
                vertices,
                triangles,
            } => build_compressed_mesh_collision(vertices, triangles, body_opts)?,
            MultiBodyShape::RawCompressedMesh { data } => {
                build_compressed_mesh_collision_from_raw(data, body_opts)?
            }
            MultiBodyShape::Compound { children } => {
                build_fo4_compound_collision(children, &body_opts)?
            }
            // Body world position is patched by the per-body merge below, so the
            // sole-builder origin can be zero here.
            MultiBodyShape::Capsule { shape } => {
                build_fo4_source_capsule_collision(shape, [0.0; 3], &body_opts)?
            }
            MultiBodyShape::SourceConvex { shape } => {
                build_fo4_source_convex_collision(shape, &body_opts)?
            }
            // Validated away above so the build dispatch can't see Sphere.
            // Keep the arm explicit so adding the real builder is a one-line
            // change instead of unwinding an unreachable.
            MultiBodyShape::Sphere { .. } => {
                unreachable!("Sphere should have been rejected by the validation pass")
            }
        };
        let hkx = HkxFile::read(&blob)?;
        let source_objects = hkx.objects();
        let psd_index = source_objects
            .iter()
            .position(|obj| obj.class_name == "hknpPhysicsSystemData")
            .ok_or_else(|| {
                HavokError::InvalidInput(
                    "single-body collision blob has no hknpPhysicsSystemData".to_string(),
                )
            })?;
        let psd = &source_objects[psd_index];

        let material_value = first_array_value(psd, "materials")?;
        let body_cinfo_value = first_array_value(psd, "bodyCinfos")?;
        let referenced_value = first_array_value(psd, "referencedObjects")?;

        if objects.is_empty() {
            objects.push(psd.clone());
        }

        let mut remap = vec![None; source_objects.len()];
        remap[psd_index] = Some(0);
        let append_start = objects.len();
        for (index, object) in source_objects.iter().enumerate() {
            if index == psd_index {
                continue;
            }
            remap[index] = Some(objects.len());
            objects.push(object.clone());
        }

        for object in &mut objects[append_start..] {
            for member in &mut object.members {
                remap_value_pointers(&mut member.value, &remap);
            }
        }

        let mut material_value = material_value.clone();
        let mut body_cinfo_value = body_cinfo_value.clone();
        let mut referenced_value = referenced_value.clone();
        remap_value_pointers(&mut material_value, &remap);
        remap_value_pointers(&mut body_cinfo_value, &remap);
        remap_value_pointers(&mut referenced_value, &remap);
        if let Some(material_members) = material_value.as_object_members_mut() {
            let meta = meta_at(body_index);
            if let Some(flags) = meta.material_flags {
                set_int_member(material_members, "flags", flags);
            }
            if let Some(trigger_type) = meta.material_trigger_type {
                set_int_member(material_members, "triggerType", i64::from(trigger_type));
            }
        }
        material_values.push(material_value);
        body_cinfo_values.push(body_cinfo_value);
        referenced_values.push(referenced_value);
    }

    // ----- Per-body metadata patch -----
    // Walk each merged bodyCinfo and rewrite the fields that single-body
    // builders cannot know about: materialId (own slot), motionId (own
    // motionCinfo entry), the world transform, and the collision filter.
    // Synthesize matching motionCinfos entries on the side.
    //
    // Motion-cinfo emission rules (validated against vanilla Safe01 + the
    // crashing bank.nif repro in crash-2026-05-11-16-21-53.log):
    //
    // - Standalone CompressedMesh bodies can keep the vanilla static pattern:
    //   motionId=HK_INVALID and no motionCinfo.
    //
    // - Shared/multi-body CompressedMesh systems need valid motionIds for every
    //   body. FO76 Vault76 railing pieces otherwise produce body 0 with
    //   HK_INVALID while later CM bodies have motionCinfos; FO4 can load the
    //   mesh, but player collision ignores the primary shape and movement can
    //   crash when hknpCompressedMeshShape interacts with terrain.
    //
    // - Static Polytope/Compound bodies follow vanilla set-dressing parity in
    //   every body-count shape: motionId=HK_INVALID and NO motionCinfo. A
    //   synthesized zero-mass cinfo makes the body half-movable: when the placed
    //   REFR has a non-unit scale FO4 wraps the convex in a runtime
    //   hknpScaledConvexShape and derives scaled mass from inverseMass=0 →
    //   convexRadius=-nan → NaN static → solver-island invalidPos cascade. The
    //   same zero-mass cinfo on a static hknpDynamicCompoundShape reproduces the
    //   NukaColaMachine01_BaseOnly crash at Fallout4.exe+13E2278.
    //
    // Mixed static+keyframed systems (Safe01/bank.nif container pattern): the
    // Static bodies are emitted as vanilla emits the safe's base — motionId=
    // HK_INVALID, no motionCinfo — handled by the `has_keyframed` guard below.
    let has_keyframed =
        (0..bodies.len()).any(|i| meta_at(i).motion_type == BodyMotionType::Keyframed);
    let all_bodies_compressed = cm_count == bodies.len();
    let needs_motion_cinfo = |body: &MultiBodyShape, motion: BodyMotionType| -> bool {
        match (body, motion) {
            (_, BodyMotionType::Keyframed) => true,
            // Mixed static+keyframed system → static bodies follow vanilla Safe01:
            // no motionCinfo, motionId=HK_INVALID.
            _ if has_keyframed => false,
            (
                MultiBodyShape::CompressedMesh { .. } | MultiBodyShape::RawCompressedMesh { .. },
                _,
            ) if bodies.len() > 1 && all_bodies_compressed => true,
            _ => false,
        }
    };

    // An articulated assembly (a constrained hanging chime, swinging sign, …) keeps
    // its dynamic chain bodies on their SOURCE layer with group bits (carried via
    // `collision_filter_info`), NOT demoted to loose clutter. Those bodies still need
    // a real dynamic motion frame like clutter does, so the constraints have live
    // bodies to swing.
    let constrained = constraints.is_some();
    // FO4 suppresses contacts inside an articulated assembly with baked ragdoll
    // part filters: every constrained body carries 0x8000 | part<<8 | layer
    // (vanilla TrapCanChimes01: anchor 0x800F, chain 0x810A..0x850A). FO76 relies
    // on runtime constraint-pair filtering instead and ships some articulated
    // sources (TireSwing02) with the bare layer — carried verbatim, the chain
    // self-collides in FO4 whenever it bends and the contact solver locks it.
    // Synthesize the vanilla numbering for constrained bodies whose source filter
    // has no part bits; sources that already carry them keep them verbatim.
    let constrained_body_indices: std::collections::HashSet<usize> = constraints
        .map(|grafted| {
            grafted
                .cinfos
                .iter()
                .flat_map(|cinfo| [cinfo.body_a as usize, cinfo.body_b as usize])
                .collect()
        })
        .unwrap_or_default();
    let mut motion_cinfo_values: Vec<HkxValue> = Vec::new();
    let mut has_dynamic_clutter = false;
    for (body_index, body_value) in body_cinfo_values.iter_mut().enumerate() {
        let meta = meta_at(body_index);
        let HkxValue::Object(body_members) = body_value else {
            return Err(HavokError::InvalidInput(
                "merged bodyCinfo is not an inline object".to_string(),
            ));
        };

        let constrained_dynamic = constrained
            && meta
                .body_flags
                .is_some_and(|flags| flags & BODY_FLAGS_DYNAMIC != 0);
        // Bodies the solver simulates dynamically: loose clutter, or a constrained
        // assembly's moving chain segments.
        let is_dynamic = meta.layer == FO4_CLUTTER_LAYER || constrained_dynamic;
        let source_filter = meta
            .collision_filter_info
            .unwrap_or_else(|| u32::from(meta.layer));
        let filter = if constrained_body_indices.contains(&body_index)
            && source_filter & RAGDOLL_PART_FILTER_FLAG == 0
        {
            i64::from(
                RAGDOLL_PART_FILTER_FLAG
                    | ((body_index as u32 & 0x7F) << 8)
                    | (source_filter & 0xFF),
            )
        } else {
            i64::from(source_filter)
        };

        set_int_member(body_members, "materialId", body_index as i64);
        set_int_member(body_members, "collisionFilterInfo", filter);
        set_int_member(body_members, "flags", body_flags_for_meta(meta, is_dynamic));
        set_vec4_member(body_members, "position", meta.position);
        set_vec4_member(body_members, "orientation", meta.orientation);

        // A dynamic body needs a real dynamic motion frame (non-zero inverse mass +
        // inverse inertia), the dynamic body flag, and a valid `motionProperties[0]`
        // to point at. The keyframed/zero cinfo (motionPropertiesId=0xFFFF,
        // inverseMass/inertia=0) makes the solver attach an infinite-mass body
        // against an empty motionProperties table and freeze the physics world on
        // cell load.
        let motion_id = if is_dynamic {
            let idx = motion_cinfo_values.len() as i64;
            motion_cinfo_values.push(build_dynamic_clutter_motion_cinfo(
                &bodies[body_index],
                meta.position,
                meta.orientation,
                meta.body_mass,
                meta.mass_distribution.as_ref(),
            ));
            has_dynamic_clutter = true;
            idx
        } else if needs_motion_cinfo(&bodies[body_index], meta.motion_type) {
            let idx = motion_cinfo_values.len() as i64;
            motion_cinfo_values.push(build_keyframed_motion_cinfo(
                meta.position,
                meta.orientation,
            ));
            idx
        } else {
            i64::from(MOTION_ID_INVALID)
        };
        set_int_member(body_members, "motionId", motion_id);
    }

    // Graft the source constraint sub-graph (hkpRagdollConstraintData + motors)
    // onto the rebuilt system, appending its objects after the shapes and building
    // `constraintCinfos` that link the output bodies. `body_a`/`body_b` arrive
    // already remapped to this call's body order.
    let mut constraint_cinfo_values: Vec<HkxValue> = Vec::new();
    if let Some(grafted) = constraints.filter(|grafted| !grafted.is_empty()) {
        let append_base = objects.len();
        let remap: Vec<Option<usize>> = (0..grafted.objects.len())
            .map(|index| Some(append_base + index))
            .collect();
        for object in &grafted.objects {
            let mut object = object.clone();
            for member in &mut object.members {
                remap_value_pointers(&mut member.value, &remap);
            }
            objects.push(object);
        }
        for cinfo in &grafted.cinfos {
            let data_index = append_base + cinfo.data_object;
            constraint_cinfo_values.push(HkxValue::Object(vec![
                HkxMember {
                    name: "constraintData".to_string(),
                    value: HkxValue::Pointer(Some(data_index)),
                },
                HkxMember {
                    name: "bodyA".to_string(),
                    value: HkxValue::U32(cinfo.body_a),
                },
                HkxMember {
                    name: "bodyB".to_string(),
                    value: HkxValue::U32(cinfo.body_b),
                },
                HkxMember {
                    name: "flags".to_string(),
                    value: HkxValue::U8(cinfo.flags as u8),
                },
            ]));
            referenced_values.push(HkxValue::Pointer(Some(data_index)));
        }
    }

    let psd = objects
        .get_mut(0)
        .ok_or_else(|| HavokError::InvalidInput("no physics system data object".to_string()))?;
    replace_array_member(psd, "materials", material_values)?;
    replace_array_member(psd, "bodyCinfos", body_cinfo_values)?;
    replace_array_member(psd, "referencedObjects", referenced_values)?;
    replace_array_member(psd, "motionCinfos", motion_cinfo_values)?;
    if !constraint_cinfo_values.is_empty() {
        replace_array_member(psd, "constraintCinfos", constraint_cinfo_values)?;
    }
    if has_dynamic_clutter {
        // Dynamic clutter bodies index `motionProperties[0]`; the rebuilt PSD
        // template ships an empty `motionProperties` array, so populate it with the
        // vanilla FO4 dynamic-clutter prototype (decoded from Abraxo / milkbottle).
        replace_array_member(
            psd,
            "motionProperties",
            vec![crate::convert::templates::motion_properties_prototype()],
        )?;
    }

    let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
    let mut blob = hkx.save();
    let compressed_mesh_markers = bodies
        .iter()
        .filter_map(|body| match body {
            MultiBodyShape::CompressedMesh { .. } => Some(0),
            MultiBodyShape::RawCompressedMesh { data } => {
                Some(data.primitive_stores_is_flat_convex)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if !compressed_mesh_markers.is_empty() {
        super::compressed_mesh::patch_fo4_compressed_mesh_flat_convex_markers(
            &mut blob,
            &compressed_mesh_markers,
        )?;
    }
    Ok(blob)
}

fn body_flags_for_meta(meta: BodyMeta, is_dynamic: bool) -> i64 {
    if is_dynamic {
        BODY_FLAGS_DYNAMIC
    } else {
        meta.body_flags.unwrap_or(0) & !BODY_FLAGS_DYNAMIC
    }
}

/// Build a vanilla-shaped `hknpMotionCinfo` for an ANIMSTATIC keyframed body.
///
/// Mirrors the layout `convert::fo76::synthesize_motion_cinfos` emits and the
/// values observed in vanilla `Safe01.nif` block 31: zero inverse mass (no
/// dynamic response), mass factor 1, infinite-ish max{Linear,Rot}, identity
/// inertia, COM = body world position, orientation = body world orientation.
fn build_keyframed_motion_cinfo(position: [f32; 4], orientation: [f32; 4]) -> HkxValue {
    HkxValue::TypedObject {
        class_name: "hknpMotionCinfo".to_string(),
        members: vec![
            HkxMember {
                name: "motionPropertiesId".to_string(),
                value: HkxValue::U16(u16::MAX),
            },
            HkxMember {
                name: "enableDeactivation".to_string(),
                value: HkxValue::Bool(true),
            },
            HkxMember {
                name: "inverseMass".to_string(),
                value: HkxValue::F32(0.0),
            },
            HkxMember {
                name: "massFactor".to_string(),
                value: HkxValue::F32(1.0),
            },
            HkxMember {
                name: "maxLinearAccelerationDistancePerStep".to_string(),
                value: HkxValue::F32(FLT_CAP),
            },
            HkxMember {
                name: "maxRotationToPreventTunneling".to_string(),
                value: HkxValue::F32(FLT_CAP),
            },
            HkxMember {
                name: "inverseInertiaLocal".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "centerOfMassWorld".to_string(),
                value: HkxValue::F32List(position.to_vec()),
            },
            HkxMember {
                name: "orientation".to_string(),
                value: HkxValue::F32List(orientation.to_vec()),
            },
            HkxMember {
                name: "linearVelocity".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "angularVelocity".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
        ],
    }
}

/// Build a vanilla-shaped *dynamic* `hknpMotionCinfo` for a CLUTTER (layer-4) body.
///
/// Unlike [`build_keyframed_motion_cinfo`] (zero inverse mass/inertia, no motion
/// properties — correct for a keyframed Safe01 door, fatal for a dynamic body),
/// this carries a real inverse mass + inverse inertia from the shape's AABB mass
/// solve and points `motionPropertiesId` at the system's `motionProperties[0]`.
/// When the source NIF carries `hknpBodyCinfo.mass`, use it directly. Otherwise
/// match vanilla FO4 loose clutter (Abraxo/milkbottle): a fixed Havok-unit body
/// mass of [`CLUTTER_BODY_MASS`] with `massFactor` reconciling it to the shape's
/// density-1.0 mass, so `inverseInertiaLocal` is rescaled to the same body mass.
fn build_dynamic_clutter_motion_cinfo(
    body: &MultiBodyShape,
    position: [f32; 4],
    orientation: [f32; 4],
    source_body_mass: Option<f32>,
    mass_dist: Option<&super::mass_properties::SourceMassDistribution>,
) -> HkxValue {
    let mp = match mass_dist {
        Some(dist) => super::mass_properties::mass_properties_from_source(dist),
        None => clutter_mass_properties(body),
    };
    let shape_mass = mp.mass.max(CLUTTER_BODY_MASS / MAX_CLUTTER_MASS_FACTOR);
    let source_body_mass = source_body_mass.filter(|mass| mass.is_finite() && *mass > 1e-9);
    let body_mass = source_body_mass.unwrap_or(CLUTTER_BODY_MASS);
    let inv_mass = 1.0 / body_mass;
    let mass_factor = if source_body_mass.is_some() {
        body_mass
    } else {
        CLUTTER_BODY_MASS / shape_mass
    };
    // The shape solve gives 1/I at `mp.mass`; inertia scales linearly with mass,
    // so rescale to the body's effective mass to stay consistent with `inverseMass`.
    // `shape_mass` is NOT the solve mass — its floor only exists to cap massFactor.
    // Rescaling by the floored value inflates inverse inertia by the floor ratio
    // for small-volume bodies (TireSwing02 rope links: 173×), leaving constrained
    // chains hyper-floppy and solver-locked rigid in-game.
    let solve_mass = if mp.mass > 0.0 { mp.mass } else { shape_mass };
    let scale = solve_mass / body_mass;
    let inv_inertia = vec![
        mp.inverse_inertia_diag[0] * scale,
        mp.inverse_inertia_diag[1] * scale,
        mp.inverse_inertia_diag[2] * scale,
        1.0,
    ];
    let rotated_center = quat_rotate(&orientation, &mp.center_of_mass);
    let com_world = vec![
        position[0] + rotated_center[0],
        position[1] + rotated_center[1],
        position[2] + rotated_center[2],
        position[3],
    ];
    HkxValue::TypedObject {
        class_name: "hknpMotionCinfo".to_string(),
        members: vec![
            HkxMember {
                name: "motionPropertiesId".to_string(),
                value: HkxValue::U16(0),
            },
            HkxMember {
                name: "enableDeactivation".to_string(),
                value: HkxValue::Bool(true),
            },
            HkxMember {
                name: "inverseMass".to_string(),
                value: HkxValue::F32(inv_mass),
            },
            HkxMember {
                name: "massFactor".to_string(),
                value: HkxValue::F32(mass_factor),
            },
            HkxMember {
                name: "maxLinearAccelerationDistancePerStep".to_string(),
                value: HkxValue::F32(FLT_CAP),
            },
            HkxMember {
                name: "maxRotationToPreventTunneling".to_string(),
                value: HkxValue::F32(FLT_CAP),
            },
            HkxMember {
                name: "inverseInertiaLocal".to_string(),
                value: HkxValue::F32List(inv_inertia),
            },
            HkxMember {
                name: "centerOfMassWorld".to_string(),
                value: HkxValue::F32List(com_world),
            },
            HkxMember {
                name: "orientation".to_string(),
                value: HkxValue::F32List(orientation.to_vec()),
            },
            HkxMember {
                name: "linearVelocity".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "angularVelocity".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
        ],
    }
}

/// AABB mass properties for a clutter body, density 1.0 (mass == hull volume,
/// floored at [`CLUTTER_MIN_MASS`]). Clutter rebuilds to a convex Polytope; the
/// Compound / mesh arms keep a sane non-zero solve for any other shape that
/// reaches the dynamic path.
fn clutter_mass_properties(body: &MultiBodyShape) -> super::mass_properties::MassProperties {
    use super::mass_properties::polytope_mass_properties;
    match body {
        MultiBodyShape::Polytope { vertices } => {
            polytope_mass_properties(vertices, clutter_mass_from_volume(vertices))
        }
        MultiBodyShape::SourcePolytope { shape } => {
            polytope_mass_properties(&shape.vertices, clutter_mass_from_volume(&shape.vertices))
        }
        MultiBodyShape::CompressedMesh { vertices, .. } => {
            polytope_mass_properties(vertices, clutter_mass_from_volume(vertices))
        }
        MultiBodyShape::Compound { children } => {
            let verts: Vec<[f32; 3]> = children
                .iter()
                .flat_map(|child| match &child.kind {
                    CompoundChildKind::Polytope { vertices } => vertices.clone(),
                    CompoundChildKind::SourcePolytope { shape } => shape.vertices.clone(),
                    CompoundChildKind::CompressedMesh { vertices, .. } => vertices.clone(),
                })
                .collect();
            polytope_mass_properties(&verts, clutter_mass_from_volume(&verts))
        }
        MultiBodyShape::Capsule { shape } => polytope_mass_properties(
            &shape.hull.vertices,
            clutter_mass_from_volume(&shape.hull.vertices),
        ),
        MultiBodyShape::SourceConvex { shape } => {
            let vertices = shape
                .vertices
                .iter()
                .map(|vertex| [vertex[0], vertex[1], vertex[2]])
                .collect::<Vec<_>>();
            polytope_mass_properties(&vertices, clutter_mass_from_volume(&vertices))
        }
        MultiBodyShape::RawCompressedMesh { .. } | MultiBodyShape::Sphere { .. } => {
            polytope_mass_properties(
                &[[-0.05, -0.05, -0.05], [0.05, 0.05, 0.05]],
                CLUTTER_MIN_MASS,
            )
        }
    }
}

fn set_int_member(members: &mut [HkxMember], name: &str, value: i64) {
    if let Some(member) = members.iter_mut().find(|m| m.name == name) {
        match &mut member.value {
            HkxValue::I8(v) => *v = value as i8,
            HkxValue::U8(v) => *v = value as u8,
            HkxValue::I16(v) => *v = value as i16,
            HkxValue::U16(v) => *v = value as u16,
            HkxValue::I32(v) => *v = value as i32,
            HkxValue::U32(v) => *v = value as u32,
            HkxValue::I64(v) => *v = value,
            HkxValue::U64(v) => *v = value as u64,
            _ => {}
        }
    }
}

fn set_vec4_member(members: &mut [HkxMember], name: &str, vec: [f32; 4]) {
    if let Some(member) = members.iter_mut().find(|m| m.name == name) {
        member.value = HkxValue::F32List(vec.to_vec());
    }
}

/// FO4 CLUTTER collision layer — loose, gravity-driven items the game simulates
/// as dynamic rigid bodies (clones the FO76→FO4 router's classification signal).
const FO4_CLUTTER_LAYER: u8 = 4;

/// Floor for a clutter body's derived mass (Havok units). Guarantees a finite,
/// stable inverse mass for thin/tiny items; FO4 vanilla loose clutter masses run
/// ~0.004–0.01, so this floor is at the low end and imperceptible.
const CLUTTER_MIN_MASS: f32 = 0.001;

/// Fixed Havok-unit body mass for dynamic clutter. Vanilla FO4 Abraxo/milkbottle
/// both ship `inverseMass = 0.1` (mass 10.0) with `massFactor` reconciling it to
/// the shape's density-1.0 mass — replicate so converted clutter is as stable as
/// vanilla rather than a near-massless jitter.
const CLUTTER_BODY_MASS: f32 = 10.0;

/// Largest clutter mass factor we let the solver see. shape_mass is floored at
/// CLUTTER_BODY_MASS / MAX_CLUTTER_MASS_FACTOR so a near-zero solved mass can't
/// make the solver explosive on contact.
const MAX_CLUTTER_MASS_FACTOR: f32 = 100.0;

/// FO4 vanilla loose clutter uses `mass == hull volume` (density 1.0) — verified
/// by decoding `hknpShapeMassProperties` on vanilla Abraxo/AlienToy (mass==volume
/// exactly). Derive the same density-1.0 mass from the convex body's AABB (the
/// mass-properties builder also approximates volume/inertia from the AABB), with a
/// floor so a thin clutter item still gets a usable, non-zero mass.
fn clutter_mass_from_volume(vertices: &[[f32; 3]]) -> f32 {
    if vertices.is_empty() {
        return CLUTTER_MIN_MASS;
    }
    let (mn, mx) = vertices.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(mn, mx), v| {
            (
                [mn[0].min(v[0]), mn[1].min(v[1]), mn[2].min(v[2])],
                [mx[0].max(v[0]), mx[1].max(v[1]), mx[2].max(v[2])],
            )
        },
    );
    let volume = (mx[0] - mn[0]) * (mx[1] - mn[1]) * (mx[2] - mn[2]);
    volume.max(CLUTTER_MIN_MASS)
}

fn options_for_body(
    opts: &BuildOptions,
    body: &MultiBodyShape,
    material_crcs: Option<&[Option<u32>]>,
    body_metas: Option<&[BodyMeta]>,
    body_index: usize,
) -> BuildOptions {
    // Per-body layer wins over the shared `opts.layer`. `opts.layer` only ever
    // describes the freshly-built body the caller is targeting — preserved
    // bodies (e.g. an existing Static CM kept across a regeneration) carry
    // their own layer in `body_metas[i].layer`. Using `opts.layer` here drives
    // `hknpBSMaterialProperties.MaterialA[i].uiFilterInfo` off the wrong layer,
    // so the body's `collisionFilterInfo` fails to match any BSMaterial entry;
    // workshop sphere casts then look the material up by filter info, get null,
    // and crash in the broadphase (Fallout4.exe+13E82D0, hknpHybridBroadPhase →
    // hknpClosestHitCollector).
    let body_layer = body_metas
        .and_then(|metas| metas.get(body_index))
        .map(|meta| meta.layer)
        .unwrap_or(opts.layer);
    let mut body_opts = opts.clone();
    body_opts.layer = body_layer;
    // Carry the source body's true mass distribution (if decoded) so the shape
    // mass-properties block uses real COM / volume / inertia instead of the AABB
    // box approximation.
    body_opts.mass_distribution = body_metas
        .and_then(|metas| metas.get(body_index))
        .and_then(|meta| meta.mass_distribution);
    // CLUTTER bodies are loose, gravity-driven items the game makes dynamic. A
    // dynamic body needs a non-zero, finite mass — mass 0 → inverse_mass 0 →
    // divide-by-zero NaN on attach, which freezes the cell's physics + sound
    // (the residual half of the loose-MISC bug: the convex shape alone wasn't
    // enough). Give clutter the FO4 vanilla density-1.0 mass so the mass-
    // properties builder fills real mass + inertia + center of mass.
    if body_layer == FO4_CLUTTER_LAYER {
        let source_body_mass = body_metas
            .and_then(|metas| metas.get(body_index))
            .and_then(|meta| meta.body_mass)
            .filter(|mass| mass.is_finite() && *mass > 1e-9);
        body_opts.mass = source_body_mass.unwrap_or_else(|| clutter_mass_properties(body).mass);
    }
    let Some(Some(material_crc)) = material_crcs.and_then(|values| values.get(body_index)) else {
        return body_opts;
    };
    body_opts.user_data = Some(u64::from(*material_crc));
    if matches!(
        body,
        MultiBodyShape::CompressedMesh { .. }
            | MultiBodyShape::RawCompressedMesh { .. }
            | MultiBodyShape::Compound { .. }
            | MultiBodyShape::SourcePolytope { .. }
            | MultiBodyShape::SourceConvex { .. }
    ) {
        body_opts.materials = vec![MaterialEntry {
            filter_info: u32::from(body_layer),
            material_crc: *material_crc,
        }];
    }
    body_opts
}

fn first_array_value<'a>(object: &'a HkxObject, name: &str) -> HavokResult<&'a HkxValue> {
    let values = object
        .members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| match &member.value {
            HkxValue::Array(values) => Some(values),
            _ => None,
        })
        .ok_or_else(|| {
            HavokError::InvalidInput(format!("hknpPhysicsSystemData.{name} is not an array"))
        })?;
    values
        .first()
        .ok_or_else(|| HavokError::InvalidInput(format!("hknpPhysicsSystemData.{name} is empty")))
}

fn replace_array_member(
    object: &mut HkxObject,
    name: &str,
    values: Vec<HkxValue>,
) -> HavokResult<()> {
    let member = object
        .members
        .iter_mut()
        .find(|member| member.name == name)
        .ok_or_else(|| {
            HavokError::InvalidInput(format!("hknpPhysicsSystemData.{name} is missing"))
        })?;
    member.value = HkxValue::Array(values);
    Ok(())
}

fn remap_value_pointers(value: &mut HkxValue, remap: &[Option<usize>]) {
    match value {
        HkxValue::Pointer(Some(index)) => {
            if let Some(mapped) = remap.get(*index) {
                *value = HkxValue::Pointer(*mapped);
            }
        }
        HkxValue::Array(values) => {
            for value in values {
                remap_value_pointers(value, remap);
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for member in members {
                remap_value_pointers(&mut member.value, remap);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CompoundChildKind;

    fn unit_cube_vertices() -> Vec<[f32; 3]> {
        vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ]
    }

    fn unit_cube_triangles() -> Vec<[u32; 3]> {
        vec![
            [0, 1, 2],
            [0, 2, 3],
            [4, 6, 5],
            [4, 7, 6],
            [0, 4, 5],
            [0, 5, 1],
            [1, 5, 6],
            [1, 6, 2],
            [2, 6, 7],
            [2, 7, 3],
            [3, 7, 4],
            [3, 4, 0],
        ]
    }

    fn find_psd(file: &HkxFile) -> &HkxObject {
        file.objects()
            .iter()
            .find(|obj| obj.class_name == "hknpPhysicsSystemData")
            .expect("hknpPhysicsSystemData missing")
    }

    fn body_member_i64(body: &HkxValue, name: &str) -> i64 {
        let HkxValue::Object(members) = body else {
            panic!("body must be inline object");
        };
        let m = members
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("body member {name} missing"));
        match &m.value {
            HkxValue::I32(v) => i64::from(*v),
            HkxValue::U32(v) => i64::from(*v),
            HkxValue::I16(v) => i64::from(*v),
            HkxValue::U16(v) => i64::from(*v),
            HkxValue::U8(v) => i64::from(*v),
            HkxValue::I8(v) => i64::from(*v),
            HkxValue::I64(v) => *v,
            HkxValue::U64(v) => *v as i64,
            other => panic!("body.{name} is not an int: {other:?}"),
        }
    }

    fn body_member_vec4(body: &HkxValue, name: &str) -> Vec<f32> {
        let HkxValue::Object(members) = body else {
            panic!("body must be inline object");
        };
        let m = members
            .iter()
            .find(|m| m.name == name)
            .expect("member missing");
        let HkxValue::F32List(values) = &m.value else {
            panic!("body.{name} is not F32List");
        };
        values.clone()
    }

    /// Vanilla Safe01 parity: a static compressed-mesh base + a keyframed
    /// (ANIMSTATIC) polytope door — the safe/container pattern. The keyframed
    /// body gets a populated motionCinfos entry and its own material; the static
    /// base gets `motionId=HK_INVALID` and NO motionCinfo, exactly like vanilla
    /// `Safe01.nif` (base motionId=0x7FFFFFFF, one motionCinfo for the door).
    ///
    /// A static base must not advertise a motion frame the broadphase then
    /// resolves during a placement cast (workshop-sweep null-deref at
    /// Fallout4.exe+13E82D0).
    #[test]
    fn clutter_polytope_body_gets_nonzero_density_mass() {
        // A CLUTTER (layer 4) convex body is a loose item the game makes dynamic;
        // it MUST carry a non-zero mass (vanilla density-1.0 = AABB volume), else
        // inverse_mass=0 → NaN on attach → cell physics/sound freeze.
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let metas = [BodyMeta {
            collision_filter_info: None,
            layer: FO4_CLUTTER_LAYER,
            ..BodyMeta::default()
        }];
        let opts = BuildOptions::default(); // mass 0.0
        let body_opts = options_for_body(&opts, &body, None, Some(&metas), 0);
        assert!(
            body_opts.mass > 0.0,
            "clutter body must get a non-zero mass"
        );
        assert_eq!(
            body_opts.mass,
            clutter_mass_from_volume(&unit_cube_vertices())
        );
    }

    #[test]
    fn dynamic_clutter_motion_cinfo_carries_real_mass_and_inertia() {
        // The keyframed/zero cinfo (inverseMass=0, inverseInertiaLocal=0,
        // motionPropertiesId=0xFFFF) froze the physics world for a body the game
        // simulates dynamically. A CLUTTER motionCinfo must carry real values and
        // index motionProperties[0].
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let cinfo =
            build_dynamic_clutter_motion_cinfo(&body, [0.0; 4], [0.0, 0.0, 0.0, 1.0], None, None);
        let HkxValue::TypedObject {
            class_name,
            members,
        } = &cinfo
        else {
            panic!("cinfo must be a TypedObject");
        };
        assert_eq!(class_name, "hknpMotionCinfo");
        let find = |name: &str| &members.iter().find(|m| m.name == name).unwrap().value;
        assert!(
            matches!(find("motionPropertiesId"), HkxValue::U16(0)),
            "must index motionProperties[0], not 0xFFFF"
        );
        let HkxValue::F32(inv_mass) = *find("inverseMass") else {
            panic!("inverseMass not F32");
        };
        assert!(inv_mass > 0.0, "dynamic body needs non-zero inverse mass");
        let HkxValue::F32List(inv_inertia) = find("inverseInertiaLocal") else {
            panic!("inverseInertiaLocal not F32List");
        };
        assert_eq!(inv_inertia.len(), 4);
        assert!(
            inv_inertia[0] > 0.0 && inv_inertia[1] > 0.0 && inv_inertia[2] > 0.0,
            "inverse inertia must be non-zero so the body can rotate, got {inv_inertia:?}"
        );
        assert_eq!(
            inv_inertia[3], 1.0,
            "inverseInertiaLocal.w sentinel must be 1.0"
        );
    }

    #[test]
    fn dynamic_clutter_motion_cinfo_uses_source_body_mass() {
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let cinfo = build_dynamic_clutter_motion_cinfo(
            &body,
            [0.0; 4],
            [0.0, 0.0, 0.0, 1.0],
            Some(2.0),
            None,
        );
        let HkxValue::TypedObject { members, .. } = &cinfo else {
            panic!("cinfo must be a TypedObject");
        };
        let f32_member = |name: &str| -> f32 {
            match &members
                .iter()
                .find(|member| member.name == name)
                .unwrap_or_else(|| panic!("{name} member missing"))
                .value
            {
                HkxValue::F32(value) => *value,
                HkxValue::Half(value) => *value,
                other => panic!("{name} is not an F32: {other:?}"),
            }
        };

        assert!((f32_member("inverseMass") - 0.5).abs() < 1e-6);
        assert!((f32_member("massFactor") - 2.0).abs() < 1e-6);
    }

    #[test]
    fn dynamic_clutter_motion_cinfo_rotates_local_center_of_mass_into_body_frame() {
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let mass_distribution = crate::collision::SourceMassDistribution {
            center_of_mass: [1.0, 0.0, 0.0],
            volume: 8.0,
            unit_inertia: [1.0, 1.0, 1.0],
            major_axis_space: [0.0, 0.0, 0.0, 1.0],
        };
        let half_sqrt_two = 0.5_f32.sqrt();
        let cinfo = build_dynamic_clutter_motion_cinfo(
            &body,
            [10.0, 20.0, 30.0, 0.0],
            [0.0, 0.0, half_sqrt_two, half_sqrt_two],
            Some(2.0),
            Some(&mass_distribution),
        );
        let HkxValue::TypedObject { members, .. } = cinfo else {
            panic!("cinfo must be a TypedObject");
        };
        let HkxValue::F32List(center) = &members
            .iter()
            .find(|member| member.name == "centerOfMassWorld")
            .expect("centerOfMassWorld member")
            .value
        else {
            panic!("centerOfMassWorld must be F32List");
        };

        assert!((center[0] - 10.0).abs() < 1e-5, "{center:?}");
        assert!((center[1] - 21.0).abs() < 1e-5, "{center:?}");
        assert!((center[2] - 30.0).abs() < 1e-5, "{center:?}");
    }

    #[test]
    fn clutter_compound_body_uses_source_body_mass() {
        let body = MultiBodyShape::Compound {
            children: vec![CompoundChild {
                transform: CompoundChild::identity_transform(),
                kind: CompoundChildKind::Polytope {
                    vertices: unit_cube_vertices(),
                },
            }],
        };
        let metas = [BodyMeta {
            collision_filter_info: None,
            layer: FO4_CLUTTER_LAYER,
            body_mass: Some(2.0),
            ..BodyMeta::default()
        }];
        let body_opts = options_for_body(&BuildOptions::default(), &body, None, Some(&metas), 0);

        assert_eq!(body_opts.mass, 2.0);
    }

    #[test]
    fn clutter_build_emits_dynamic_body_flag_and_motion_properties() {
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let metas = [BodyMeta {
            collision_filter_info: None,
            layer: FO4_CLUTTER_LAYER,
            ..BodyMeta::default()
        }];
        let blob =
            build_fo4_multi_body_collision(&[body], &BuildOptions::default(), None, Some(&metas))
                .expect("build clutter body");
        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);

        let body_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("bodyCinfos not array"),
        };
        assert_eq!(
            body_member_i64(&body_cinfos[0], "flags"),
            128,
            "clutter body must be flagged dynamic"
        );
        assert_eq!(body_member_i64(&body_cinfos[0], "motionId"), 0);

        // motionProperties must be populated (the game indexes [0]); an empty table
        // is what froze the physics world on cell attach.
        let mp_len = match &psd
            .members
            .iter()
            .find(|m| m.name == "motionProperties")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.len(),
            _ => panic!("motionProperties not array"),
        };
        assert_eq!(
            mp_len, 1,
            "dynamic clutter must ship a motionProperties entry"
        );

        let motion_properties = match &psd
            .members
            .iter()
            .find(|m| m.name == "motionProperties")
            .unwrap()
            .value
        {
            HkxValue::Array(values) => values,
            _ => unreachable!("motionProperties must be an array"),
        };
        let HkxValue::Object(members) = &motion_properties[0] else {
            panic!("motionProperties[0] must be an inline object");
        };
        let gravity = members
            .iter()
            .find(|member| member.name == "gravityFactor")
            .map(|member| &member.value)
            .unwrap_or_else(|| panic!("gravityFactor missing"));
        assert!(
            matches!(gravity, HkxValue::F32(value) if (*value - 1.0).abs() < 1e-6),
            "dynamic clutter gravityFactor must round-trip as 1.0, got {gravity:?}"
        );
    }

    #[test]
    fn static_trigger_body_preserves_source_body_and_material_flags() {
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let metas = [BodyMeta {
            collision_filter_info: None,
            layer: 12,
            body_flags: Some(16),
            material_flags: Some(1 << 21),
            material_trigger_type: Some(2),
            ..BodyMeta::default()
        }];
        let blob =
            build_fo4_multi_body_collision(&[body], &BuildOptions::default(), None, Some(&metas))
                .expect("build trigger body");
        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);

        let body_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("bodyCinfos not array"),
        };
        assert_eq!(body_member_i64(&body_cinfos[0], "collisionFilterInfo"), 12);
        assert_eq!(body_member_i64(&body_cinfos[0], "flags"), 16);
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            i64::from(MOTION_ID_INVALID)
        );

        let materials = match &psd
            .members
            .iter()
            .find(|m| m.name == "materials")
            .unwrap()
            .value
        {
            HkxValue::Array(values) => values,
            _ => panic!("materials not array"),
        };
        assert_eq!(body_member_i64(&materials[0], "flags"), 1 << 21);
        assert_eq!(body_member_i64(&materials[0], "triggerType"), 2);

        let motion_cinfo_len = match &psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.len(),
            _ => panic!("motionCinfos not array"),
        };
        assert_eq!(motion_cinfo_len, 0);
    }

    #[test]
    fn grafted_constraints_emit_constraint_cinfos_and_preserve_filter() {
        use crate::collision::{GraftCinfo, GraftedConstraints};

        // Two dynamic chain bodies linked by one constraint — the shape of an
        // articulated trap segment. Regression guard for the FO76 bone-chime fix:
        // constraints (and their full 0x81xx group filter) must survive the
        // re-encode instead of being dropped.
        let bodies = [
            MultiBodyShape::Polytope {
                vertices: unit_cube_vertices(),
            },
            MultiBodyShape::Polytope {
                vertices: unit_cube_vertices(),
            },
        ];
        let metas = [
            BodyMeta {
                collision_filter_info: Some(0x810a),
                layer: 10,
                body_flags: Some(BODY_FLAGS_DYNAMIC),
                ..BodyMeta::default()
            },
            BodyMeta {
                collision_filter_info: Some(0x820a),
                layer: 10,
                body_flags: Some(BODY_FLAGS_DYNAMIC),
                ..BodyMeta::default()
            },
        ];
        let constraint_data = HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hkpPositionConstraintMotor".to_string(),
            members: vec![
                HkxMember {
                    name: "type".to_string(),
                    value: HkxValue::I32(3),
                },
                HkxMember {
                    name: "tau".to_string(),
                    value: HkxValue::F32(0.8),
                },
            ],
        };
        let grafted = GraftedConstraints {
            objects: vec![constraint_data],
            cinfos: vec![GraftCinfo {
                body_a: 1,
                body_b: 0,
                data_object: 0,
                flags: 0,
            }],
        };

        let blob = build_fo4_multi_body_collision_with_constraints(
            &bodies,
            &BuildOptions::default(),
            None,
            Some(&metas),
            Some(&grafted),
        )
        .expect("build constrained assembly");
        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);

        let body_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("bodyCinfos not array"),
        };
        assert_eq!(
            body_member_i64(&body_cinfos[0], "collisionFilterInfo"),
            0x810a,
            "full source filter (group bits) must be preserved, not demoted to layer"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "collisionFilterInfo"),
            0x820a
        );

        let constraint_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "constraintCinfos")
            .expect("constraintCinfos must be emitted")
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("constraintCinfos not array"),
        };
        assert_eq!(constraint_cinfos.len(), 1);
        let HkxValue::Object(members) = &constraint_cinfos[0] else {
            panic!("constraint cinfo must be inline object");
        };
        let field = |name: &str| members.iter().find(|m| m.name == name).map(|m| &m.value);
        assert!(matches!(field("bodyA"), Some(HkxValue::U32(1))));
        assert!(matches!(field("bodyB"), Some(HkxValue::U32(0))));
        assert!(
            matches!(field("constraintData"), Some(HkxValue::Pointer(Some(_)))),
            "constraintData must resolve to the grafted object"
        );
        assert!(
            file.objects()
                .iter()
                .any(|o| o.class_name == "hkpPositionConstraintMotor"),
            "grafted constraint object must be appended to the system"
        );
    }

    #[test]
    fn constrained_bodies_without_source_part_bits_get_vanilla_part_filters() {
        use crate::collision::{GraftCinfo, GraftedConstraints};

        // TireSwing02 shape: FO76 ships the anchor (layer 15) and chain (layer 4)
        // with BARE layer filters, relying on runtime constraint-pair filtering
        // FO4 doesn't do. The re-encode must synthesize the vanilla part numbering
        // (0x8000 | body_index<<8 | layer) for every constrained body; an
        // unconstrained body in the same system keeps its bare filter.
        let bodies = [
            MultiBodyShape::Polytope {
                vertices: unit_cube_vertices(),
            },
            MultiBodyShape::Polytope {
                vertices: unit_cube_vertices(),
            },
            MultiBodyShape::Polytope {
                vertices: unit_cube_vertices(),
            },
        ];
        let metas = [
            BodyMeta {
                collision_filter_info: Some(0x000f),
                layer: 15,
                ..BodyMeta::default()
            },
            BodyMeta {
                collision_filter_info: Some(0x0004),
                layer: 4,
                body_flags: Some(BODY_FLAGS_DYNAMIC),
                ..BodyMeta::default()
            },
            BodyMeta {
                collision_filter_info: Some(0x0004),
                layer: 4,
                body_flags: Some(BODY_FLAGS_DYNAMIC),
                ..BodyMeta::default()
            },
        ];
        let constraint_data = HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hkpRagdollConstraintData".to_string(),
            members: vec![HkxMember {
                name: "userData".to_string(),
                value: HkxValue::U64(0),
            }],
        };
        let grafted = GraftedConstraints {
            objects: vec![constraint_data],
            cinfos: vec![GraftCinfo {
                body_a: 1,
                body_b: 0,
                data_object: 0,
                flags: 0,
            }],
        };

        let blob = build_fo4_multi_body_collision_with_constraints(
            &bodies,
            &BuildOptions::default(),
            None,
            Some(&metas),
            Some(&grafted),
        )
        .expect("build constrained assembly");
        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let body_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("bodyCinfos not array"),
        };
        assert_eq!(
            body_member_i64(&body_cinfos[0], "collisionFilterInfo"),
            0x800f,
            "constrained anchor must gain the vanilla part filter"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "collisionFilterInfo"),
            0x8104,
            "constrained chain body must gain part<<8 over its source layer"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[2], "collisionFilterInfo"),
            0x0004,
            "unconstrained body must keep its bare source filter"
        );
    }

    #[test]
    fn source_mass_distribution_inertia_solves_at_body_mass() {
        use crate::collision::mass_properties::SourceMassDistribution;

        // TireSwing02 rope link: volume 0.000577 sits far below the massFactor
        // floor (0.1). The inertia solve happens at the volume-mass, so the
        // rescale to the source body mass must use that same mass — flooring it
        // first inflated inverse inertia 173× and locked constrained chains.
        let dist = SourceMassDistribution {
            center_of_mass: [0.0, 0.0, 0.05],
            volume: 0.000577,
            unit_inertia: [0.018935, 0.019039, 0.000386],
            major_axis_space: [0.0, 0.0, 0.0, 1.0],
        };
        let body_mass = 2.0_f32;
        let metas = [BodyMeta {
            layer: FO4_CLUTTER_LAYER,
            body_flags: Some(BODY_FLAGS_DYNAMIC),
            body_mass: Some(body_mass),
            mass_distribution: Some(dist),
            ..BodyMeta::default()
        }];
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let blob =
            build_fo4_multi_body_collision(&[body], &BuildOptions::default(), None, Some(&metas))
                .expect("build dynamic body");
        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let motion_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("motionCinfos not array"),
        };
        assert_eq!(motion_cinfos.len(), 1);
        let HkxValue::Object(members) = &motion_cinfos[0] else {
            panic!("motion cinfo must be inline object");
        };
        let find = |name: &str| members.iter().find(|m| m.name == name).map(|m| &m.value);
        let HkxValue::F32List(inv_inertia) = find("inverseInertiaLocal").unwrap() else {
            panic!("inverseInertiaLocal not F32List");
        };
        for (axis, unit) in dist.unit_inertia.iter().enumerate() {
            let expected = 1.0 / (unit * body_mass);
            let actual = inv_inertia[axis];
            assert!(
                (actual - expected).abs() / expected < 1e-3,
                "axis {axis}: inverse inertia {actual} must solve at body mass (expected {expected})"
            );
        }
        assert!(
            matches!(find("inverseMass"), Some(HkxValue::F32(v)) if (*v - 0.5).abs() < 1e-6),
            "inverseMass must stay 1/body_mass"
        );
    }

    #[test]
    fn non_clutter_body_does_not_preserve_dynamic_body_flag() {
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let metas = [BodyMeta {
            collision_filter_info: None,
            layer: 12,
            body_flags: Some(BODY_FLAGS_DYNAMIC | 16),
            ..BodyMeta::default()
        }];
        let blob =
            build_fo4_multi_body_collision(&[body], &BuildOptions::default(), None, Some(&metas))
                .expect("build trigger body");
        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let body_cinfos = match &psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap()
            .value
        {
            HkxValue::Array(v) => v.clone(),
            _ => panic!("bodyCinfos not array"),
        };
        assert_eq!(body_member_i64(&body_cinfos[0], "flags"), 16);
    }

    #[test]
    fn static_polytope_body_keeps_zero_mass() {
        // Layer 1 (STATIC) must stay mass 0 — only movable CLUTTER gets a mass.
        let body = MultiBodyShape::Polytope {
            vertices: unit_cube_vertices(),
        };
        let metas = [BodyMeta {
            collision_filter_info: None,
            layer: 1,
            ..BodyMeta::default()
        }];
        let opts = BuildOptions::default();
        let body_opts = options_for_body(&opts, &body, None, Some(&metas), 0);
        assert_eq!(body_opts.mass, 0.0);
    }

    #[test]
    fn keyframed_body_emits_motion_cinfo_and_unique_material() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions::default();
        let metas = vec![
            BodyMeta {
                collision_filter_info: None,
                layer: 1,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.0, 0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
                motion_type: BodyMotionType::Static,
                body_mass: None,
                mass_distribution: None,
            },
            BodyMeta {
                collision_filter_info: None,
                layer: 2,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.3, -0.3, 0.6, 0.0],
                orientation: [0.0, 0.0, -0.707, 0.707],
                motion_type: BodyMotionType::Keyframed,
                body_mass: None,
                mass_distribution: None,
            },
        ];
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris.clone(),
                },
                MultiBodyShape::Polytope { vertices: verts },
            ],
            &opts,
            None,
            Some(&metas),
        )
        .expect("build keyframed multi-body");

        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);

        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!("bodyCinfos not array"),
            })
            .expect("bodyCinfos present");
        assert_eq!(body_cinfos.len(), 2, "two bodies expected");

        // Body 0: Static CM base — vanilla Safe01 emits motionId=HK_INVALID and
        // NO motionCinfo for it.
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "static CM base in a keyframed system must be motionId=HK_INVALID (Safe01 parity)"
        );
        assert_eq!(body_member_i64(&body_cinfos[0], "materialId"), 0);
        assert_eq!(body_member_i64(&body_cinfos[0], "collisionFilterInfo"), 1);

        // Body 1: Keyframed door — motionId points at the single motionCinfos[0].
        assert_eq!(
            body_member_i64(&body_cinfos[1], "motionId"),
            0,
            "keyframed body must point at the sole motionCinfos[0]"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "materialId"),
            1,
            "body 1 must point at materials[1], not the shared slot 0"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "collisionFilterInfo"),
            2,
            "keyframed body must carry its own layer (ANIMSTATIC=2)"
        );
        let pos = body_member_vec4(&body_cinfos[1], "position");
        assert!((pos[0] - 0.3).abs() < 1e-5 && (pos[2] - 0.6).abs() < 1e-5);

        // Only the keyframed door gets a motionCinfo (vanilla Safe01 has exactly 1).
        let motion_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!("motionCinfos not array"),
            })
            .expect("motionCinfos present");
        assert_eq!(
            motion_cinfos.len(),
            1,
            "only the keyframed body emits a motionCinfo (Safe01 parity); \
             the static base must not"
        );
        // Reader resolves TypedObject -> Object once the descriptor knows the
        // array element class; accept either shape.
        let members: &[HkxMember] = match &motion_cinfos[0] {
            HkxValue::Object(m) | HkxValue::TypedObject { members: m, .. } => m,
            other => panic!("motionCinfo must be an inline struct: {other:?}"),
        };
        let com_member = members
            .iter()
            .find(|m| m.name == "centerOfMassWorld")
            .expect("centerOfMassWorld present");
        let HkxValue::F32List(com) = &com_member.value else {
            panic!("centerOfMassWorld must be F32List");
        };
        assert!((com[0] - 0.3).abs() < 1e-5);
        assert!((com[2] - 0.6).abs() < 1e-5);
    }

    /// Safe01 parity, pinned independently of geometry/material details: in a
    /// static-base + keyframed-door system the motionCinfos count must equal the
    /// number of keyframed bodies (1), and every Static body must carry
    /// motionId=HK_INVALID. This is the invariant whose violation re-armed the
    /// workshop-sweep CTD at Fallout4.exe+13E82D0 across multiple rounds; guard
    /// it by itself so a future refactor that re-adds a static-body motionCinfo
    /// fails here with an unambiguous name.
    #[test]
    fn safe01_parity_static_base_emits_no_motion_cinfo() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions::default();
        let metas = vec![
            BodyMeta {
                collision_filter_info: None,
                layer: 1,
                motion_type: BodyMotionType::Static,
                ..BodyMeta::default()
            },
            BodyMeta {
                collision_filter_info: None,
                layer: 2,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [1.0, 2.0, 3.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
                motion_type: BodyMotionType::Keyframed,
                body_mass: None,
                mass_distribution: None,
            },
        ];
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris,
                },
                MultiBodyShape::Polytope { vertices: verts },
            ],
            &opts,
            None,
            Some(&metas),
        )
        .expect("build static-base + keyframed-door system");

        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);

        let motion_cinfos_len = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.len(),
                _ => panic!("motionCinfos not array"),
            })
            .expect("motionCinfos present");
        assert_eq!(
            motion_cinfos_len, 1,
            "exactly one motionCinfo (the keyframed door), matching vanilla Safe01"
        );

        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!("bodyCinfos not array"),
            })
            .expect("bodyCinfos present");
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "static base must be HK_INVALID, never a synthesized motion frame"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "motionId"),
            0,
            "keyframed door points at the sole motionCinfos[0]"
        );
    }

    /// Force the writer to actually run on `blob` by dirtying the model.
    /// `HkxFile::save()` echoes `source_bytes` verbatim when `model_dirty` is
    /// false, which makes the naive read→save→read pattern silently bypass
    /// the writer.
    fn force_writer_roundtrip(blob: &[u8]) -> Vec<u8> {
        let mut file = HkxFile::read(blob).expect("parse blob for forced writer roundtrip");
        let _ = file.objects_mut(); // marks model_dirty=true → writer runs on save
        file.save()
    }

    /// Single-body packfile: force the writer through HkxFile::save and confirm
    /// the Half-typed material members survive (counterpart to the multi-body
    /// regression).
    #[test]
    fn single_body_writer_preserves_material_halves() {
        use crate::collision::polytope::build_fo4_polytope_collision;
        let blob =
            build_fo4_polytope_collision(&unit_cube_vertices(), &BuildOptions::default()).unwrap();
        let out = force_writer_roundtrip(&blob);
        let file2 = HkxFile::read(&out).unwrap();
        let psd = file2
            .objects()
            .iter()
            .find(|o| o.class_name == "hknpPhysicsSystemData")
            .unwrap();
        let mat_arr = psd.members.iter().find(|m| m.name == "materials").unwrap();
        let HkxValue::Array(arr) = &mat_arr.value else {
            panic!("materials not array")
        };
        let mems = match &arr[0] {
            HkxValue::Object(m) | HkxValue::TypedObject { members: m, .. } => m,
            other => panic!("material[0] not inline: {other:?}"),
        };
        let read_f = |name: &str| -> f32 {
            mems.iter()
                .find(|m| m.name == name)
                .map(|m| match m.value {
                    HkxValue::Half(v) => v,
                    HkxValue::F32(v) => v,
                    _ => -1.0,
                })
                .unwrap_or(-1.0)
        };
        assert!(
            read_f("dynamicFriction") > 0.0,
            "writer lost dynamicFriction"
        );
        assert!(
            read_f("weldingTolerance") > 0.0,
            "writer lost weldingTolerance"
        );
    }

    /// Diagnostic: inspect what HkxFile actually parses for a single polytope's
    /// material[0] members. Logged via --nocapture to debug the friction-loss
    /// issue.
    #[test]
    fn probe_single_polytope_material_members() {
        use crate::collision::polytope::build_fo4_polytope_collision;
        let opts = BuildOptions::default();
        let blob = build_fo4_polytope_collision(&unit_cube_vertices(), &opts).unwrap();
        let file = HkxFile::read(&blob).unwrap();
        let psd = file
            .objects()
            .iter()
            .find(|o| o.class_name == "hknpPhysicsSystemData")
            .unwrap();
        let mat_arr = psd.members.iter().find(|m| m.name == "materials").unwrap();
        let HkxValue::Array(arr) = &mat_arr.value else {
            panic!()
        };
        for (i, m) in arr.iter().enumerate() {
            let mems = match m {
                HkxValue::Object(m) | HkxValue::TypedObject { members: m, .. } => m,
                _ => panic!("material {i} not inline"),
            };
            println!("material[{i}]:");
            for member in mems {
                println!("  {} = {:?}", member.name, member.value);
            }
        }
    }

    /// Mixed-shape static multi-body must follow vanilla static set-dressing:
    /// no zero-mass motionCinfos. FO4 vending machines use HK_INVALID for their
    /// static hknpDynamicCompoundShape and layer-49 convex body.
    #[test]
    fn mixed_static_cm_and_polytope_do_not_emit_motion_cinfos() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions::default();
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris,
                },
                MultiBodyShape::Polytope { vertices: verts },
            ],
            &opts,
            None,
            None, // no metas → defaults: all static
        )
        .expect("build static multi-body");

        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let motion_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.len(),
                _ => panic!("motionCinfos not array"),
            })
            .expect("motionCinfos present");
        assert_eq!(
            motion_cinfos, 0,
            "static mixed CM + polytope must not emit zero-mass motionCinfos"
        );

        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!(),
            })
            .unwrap();
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "static mixed CM body must be HK_INVALID"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "static polytope body must be HK_INVALID"
        );
    }

    #[test]
    fn multi_compressed_mesh_bodies_share_system_and_emit_motion_cinfos() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions::default();
        let metas = vec![
            BodyMeta {
                collision_filter_info: None,
                layer: 1,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.0, 0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
                motion_type: BodyMotionType::Static,
                body_mass: None,
                mass_distribution: None,
            },
            BodyMeta {
                collision_filter_info: None,
                layer: 31,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.0, 0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
                motion_type: BodyMotionType::Static,
                body_mass: None,
                mass_distribution: None,
            },
            BodyMeta {
                collision_filter_info: None,
                layer: 3,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.0, 0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
                motion_type: BodyMotionType::Static,
                body_mass: None,
                mass_distribution: None,
            },
        ];
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris.clone(),
                },
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris.clone(),
                },
                MultiBodyShape::CompressedMesh {
                    vertices: verts,
                    triangles: tris,
                },
            ],
            &opts,
            None,
            Some(&metas),
        )
        .expect("build shared multi-compressed-mesh collision");

        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!("bodyCinfos not array"),
            })
            .expect("bodyCinfos present");
        assert_eq!(body_cinfos.len(), 3, "three shared bodies expected");
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            0,
            "primary shared static CM must point at motionCinfos[0]"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "motionId"),
            1,
            "secondary CM must point at motionCinfos[1]"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[2], "motionId"),
            2,
            "third CM must point at motionCinfos[2]"
        );
        assert_eq!(body_member_i64(&body_cinfos[1], "collisionFilterInfo"), 31);
        assert_eq!(body_member_i64(&body_cinfos[2], "collisionFilterInfo"), 3);

        let motion_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.len(),
                _ => panic!("motionCinfos not array"),
            })
            .expect("motionCinfos present");
        assert_eq!(
            motion_cinfos, 3,
            "multi-CM systems need one motionCinfo per shared CM body"
        );
    }

    #[test]
    fn mixed_static_compressed_and_compound_bodies_do_not_emit_motion_cinfos() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions::default();
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris.clone(),
                },
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris,
                },
                MultiBodyShape::Compound {
                    children: vec![CompoundChild {
                        transform: CompoundChild::identity_transform(),
                        kind: CompoundChildKind::Polytope { vertices: verts },
                    }],
                },
            ],
            &opts,
            None,
            None,
        )
        .expect("build shared compressed+compound collision");

        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!("bodyCinfos not array"),
            })
            .expect("bodyCinfos present");
        assert_eq!(body_cinfos.len(), 3, "three shared bodies expected");
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "primary static CM in a mixed system must be HK_INVALID"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[1], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "secondary static CM in a mixed system must be HK_INVALID"
        );
        assert_eq!(
            body_member_i64(&body_cinfos[2], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "static compound body must be HK_INVALID"
        );

        let motion_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.len(),
                _ => panic!("motionCinfos not array"),
            })
            .expect("motionCinfos present");
        assert_eq!(
            motion_cinfos, 0,
            "static mixed CM + compound systems must not emit zero-mass motionCinfos"
        );
    }

    /// Standalone (single-body) static polytope must match vanilla set-dressing
    /// (BarrelFlammable.nif; Safe01/bank.nif bases): motionId=HK_INVALID and NO
    /// motionCinfo. A synthesized zero-mass cinfo makes a non-unit-scaled placed
    /// ref NaN — FO4 wraps the convex in a runtime hknpScaledConvexShape and
    /// derives scaled mass from inverseMass=0 → convexRadius=-nan → solver-island
    /// invalidPos cascade (live capture: Firewood01 0.9, Fancy_Chandelier 0.69).
    #[test]
    fn single_static_polytope_is_hk_invalid_without_motion_cinfo() {
        let verts = unit_cube_vertices();
        let opts = BuildOptions::default();
        let blob = build_fo4_multi_body_collision(
            &[MultiBodyShape::Polytope { vertices: verts }],
            &opts,
            None,
            None, // defaults to Static
        )
        .expect("build standalone static polytope");

        let file = HkxFile::read(&blob).expect("parse");
        let psd = find_psd(&file);
        let motion_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.len(),
                _ => panic!("motionCinfos not array"),
            })
            .expect("motionCinfos present");
        assert_eq!(
            motion_cinfos, 0,
            "standalone static polytope must emit NO motionCinfo (vanilla parity); \
             a synthesized cinfo NaNs scaled refs via runtime hknpScaledConvexShape"
        );

        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .map(|m| match &m.value {
                HkxValue::Array(v) => v.clone(),
                _ => panic!(),
            })
            .unwrap();
        assert_eq!(body_cinfos.len(), 1);
        assert_eq!(
            body_member_i64(&body_cinfos[0], "motionId"),
            i64::from(MOTION_ID_INVALID),
            "standalone static polytope's body must be HK_INVALID (no motion frame)"
        );
    }

    /// Reproduce: bank.nif crash @ FO4+13E82D0. The merged multi-body PSD must
    /// not drop the hkHalf material fields (dynamicFriction, weldingTolerance,
    /// massChangerHeavyObjectFactor, disablingCollisionsBetweenCvxCvxDynamicObjectsDistance, ...).
    /// Vanilla Safe01.nif keeps them at 1.75 / 1.32 / 1.875 / 2.3125; our
    /// merged blob drops them all to 0.0 → Havok broadphase null-derefs on
    /// workshop sweep / collision filter init.
    #[test]
    fn multi_body_preserves_material_half_fields() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions::default();
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris,
                },
                MultiBodyShape::Polytope { vertices: verts },
            ],
            &opts,
            None,
            None,
        )
        .expect("build multi-body");

        let file = HkxFile::read(&blob).expect("parse merged blob");
        let psd = find_psd(&file);
        let mat_arr = psd
            .members
            .iter()
            .find(|m| m.name == "materials")
            .expect("materials present");
        let HkxValue::Array(arr) = &mat_arr.value else {
            panic!("materials not array");
        };
        for (i, mat) in arr.iter().enumerate() {
            let mems = match mat {
                HkxValue::Object(m) | HkxValue::TypedObject { members: m, .. } => m,
                _ => panic!("material[{i}] not inline"),
            };
            let read_f = |name: &str| -> f32 {
                mems.iter()
                    .find(|m| m.name == name)
                    .map(|m| match m.value {
                        HkxValue::Half(v) => v,
                        HkxValue::F32(v) => v,
                        _ => -1.0,
                    })
                    .unwrap_or(-1.0)
            };
            let dyn_fric = read_f("dynamicFriction");
            let stat_fric = read_f("staticFriction");
            let weld = read_f("weldingTolerance");
            println!("merged material[{i}]: dynFric={dyn_fric} statFric={stat_fric} weld={weld}");
            assert!(
                dyn_fric > 0.0,
                "merged material[{i}].dynamicFriction lost in writer round-trip: {dyn_fric}"
            );
            assert!(
                weld > 0.0,
                "merged material[{i}].weldingTolerance lost in writer round-trip: {weld}"
            );
        }
    }

    /// Regression: `hknpBSMaterialProperties.MaterialA[i].uiFilterInfo` must
    /// equal the body's `collisionFilterInfo`, which comes from
    /// `body_metas[i].layer` — NOT the shared `opts.layer`. Mismatch leaves
    /// the body's material unresolvable (Havok scans `MaterialA` for
    /// `filterInfo == body.collisionFilterInfo`); a null material lookup then
    /// crashes the broadphase on workshop sphere casts (Fallout4.exe+13E82D0).
    ///
    /// Setup: opts.layer=2 (the new polytope), body 0 = CompressedMesh
    /// preserved on layer 1; body 0's MaterialA entry must not get uiFilterInfo=2
    /// while its body's collisionFilterInfo=1.
    #[test]
    fn per_body_bs_material_filter_info_matches_body_layer() {
        let verts = unit_cube_vertices();
        let tris = unit_cube_triangles();
        let opts = BuildOptions {
            layer: 2, // shared "new body" layer (would have been polytope)
            ..BuildOptions::default()
        };
        let metas = vec![
            BodyMeta {
                collision_filter_info: None,
                layer: 1, // preserved CompressedMesh on STATIC
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.0, 0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
                motion_type: BodyMotionType::Static,
                body_mass: None,
                mass_distribution: None,
            },
            BodyMeta {
                collision_filter_info: None,
                layer: 2, // new polytope on ANIMSTATIC
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [0.3, -0.3, 0.6, 0.0],
                orientation: [0.0, 0.0, -0.707, 0.707],
                motion_type: BodyMotionType::Keyframed,
                body_mass: None,
                mass_distribution: None,
            },
        ];
        let material_crcs: Vec<Option<u32>> = vec![Some(0xC0EB_623D), Some(0xC0EB_623D)];
        let blob = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::CompressedMesh {
                    vertices: verts.clone(),
                    triangles: tris,
                },
                MultiBodyShape::Polytope { vertices: verts },
            ],
            &opts,
            Some(&material_crcs),
            Some(&metas),
        )
        .expect("build multi-body with per-body crcs + metas");

        let file = HkxFile::read(&blob).expect("parse merged blob");
        let psd = find_psd(&file);

        // The merged BS material array (in hknpBSMaterialProperties.MaterialA)
        // is reachable through the per-body hkRefCountedProperties; for the
        // smoke test we read the body 0 source's surviving filter_info via
        // havok_collision_summary, which decodes both bodies' bs_materials.
        let summary_json = crate::api::havok_collision_summary(&blob).expect("summary");
        let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
        let bodies = summary["bodies"].as_array().expect("bodies array");
        assert_eq!(bodies.len(), 2);

        // Body 0 (CompressedMesh): collision_filter_info=1, BSMaterial filter
        // info must also be 1 — they must match for Havok to resolve material.
        let body0_filter = bodies[0]["collision_filter_info"].as_u64().unwrap();
        let body0_bs_filter = bodies[0]["bs_materials"][0]["filter_info"]
            .as_u64()
            .expect("body 0 BSMaterial filter_info");
        assert_eq!(body0_filter, 1, "body 0 collisionFilterInfo");
        assert_eq!(
            body0_bs_filter, body0_filter,
            "body 0 BSMaterial filter_info must match body's collisionFilterInfo; \
             mismatched values (uiFilterInfo={body0_bs_filter}, filter={body0_filter}) \
             reproduce the workshop-sweep broadphase null-deref CTD"
        );

        // PSD-level materials array: each body owns its own slot, and slot i's
        // filter info equals body i's layer. (Polytopes don't carry their own
        // BSMaterial in vanilla, but the cinfo's collisionFilterInfo must still
        // match its meta.)
        let body1_filter = bodies[1]["collision_filter_info"].as_u64().unwrap();
        assert_eq!(body1_filter, 2, "body 1 collisionFilterInfo");

        // Sanity: ensure body 0 didn't accidentally get the polytope's layer.
        assert_ne!(
            body0_filter, body1_filter,
            "bodies must keep distinct collision layers"
        );
        let _ = psd;
    }

    /// Regression: hknpMaterial carries two `hkUFloat8` inline structs
    /// (`triggerManifoldTolerance` at offset 17, `softContactSeperationVelocity`
    /// at offset 44). The descriptor must declare a single 1-byte UINT8 member
    /// — if it ever regresses (missing XML, wrong type), `calc_inline_struct_size`
    /// returns 0 and every half field after offset 17 shifts, reproducing the
    /// same crash class as the original half-writer bug.
    #[test]
    fn writer_preserves_hk_ufloat8_inline_struct() {
        use crate::collision::polytope::build_fo4_polytope_collision;
        let blob =
            build_fo4_polytope_collision(&unit_cube_vertices(), &BuildOptions::default()).unwrap();

        let read_ufloat8 = |bytes: &[u8], field: &str| -> Option<u8> {
            let file = HkxFile::read(bytes).ok()?;
            let psd = file
                .objects()
                .iter()
                .find(|o| o.class_name == "hknpPhysicsSystemData")?;
            let mat_arr = psd.members.iter().find(|m| m.name == "materials")?;
            let HkxValue::Array(arr) = &mat_arr.value else {
                return None;
            };
            let members = arr[0].as_object_members()?;
            let m = members.iter().find(|m| m.name == field)?;
            let inner = m.value.as_object_members()?;
            let val_member = inner.iter().find(|m| m.name == "value")?;
            match val_member.value {
                HkxValue::U8(v) => Some(v),
                _ => None,
            }
        };

        let pre_trigger = read_ufloat8(&blob, "triggerManifoldTolerance");
        let pre_softvel = read_ufloat8(&blob, "softContactSeperationVelocity");
        assert!(
            pre_trigger.is_some() && pre_softvel.is_some(),
            "hkUFloat8 descriptor missing — reader produced None for triggerManifoldTolerance={pre_trigger:?} / softContactSeperationVelocity={pre_softvel:?}",
        );

        let out = force_writer_roundtrip(&blob);
        let post_trigger = read_ufloat8(&out, "triggerManifoldTolerance");
        let post_softvel = read_ufloat8(&out, "softContactSeperationVelocity");
        assert_eq!(
            post_trigger, pre_trigger,
            "writer dropped hkUFloat8 triggerManifoldTolerance"
        );
        assert_eq!(
            post_softvel, pre_softvel,
            "writer dropped hkUFloat8 softContactSeperationVelocity"
        );
    }

    /// Single-body sphere routes through the short-circuit path in
    /// `build_fo4_multi_body_collision` and returns the raw sphere blob
    /// directly so the descriptor-unknown trailer bytes (0x30 marker, 0x4C
    /// trailer float) survive — they don't round-trip through HkxFile::save.
    #[test]
    fn single_body_sphere_preserves_trailer_bytes() {
        let metas = vec![BodyMeta {
            collision_filter_info: None,
            layer: 5,
            body_flags: None,
            material_flags: None,
            material_trigger_type: None,
            position: [1.0, 2.0, 3.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            motion_type: BodyMotionType::Static,
            body_mass: None,
            mass_distribution: None,
        }];
        let blob = build_fo4_multi_body_collision(
            &[MultiBodyShape::Sphere {
                radius: 0.5,
                position: [0.0, 0.0, 0.0], // overridden by meta
            }],
            &BuildOptions::default(),
            None,
            Some(&metas),
        )
        .expect("build single-body sphere via multi-body API");

        let file = HkxFile::read(&blob).expect("parse sphere packfile");
        assert!(
            file.objects()
                .iter()
                .any(|o| o.class_name == "hknpSphereShape"),
            "sphere shape missing from multi-body sphere blob"
        );

        // BodyMeta position must override the radius/position param on the
        // enum so the caller's world transform wins.
        let psd = file
            .objects()
            .iter()
            .find(|o| o.class_name == "hknpPhysicsSystemData")
            .expect("PSD present");
        let body_cinfos = psd
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .expect("bodyCinfos");
        let HkxValue::Array(arr) = &body_cinfos.value else {
            panic!("bodyCinfos not array");
        };
        let pos = body_member_vec4(&arr[0], "position");
        assert!((pos[0] - 1.0).abs() < 1e-5 && (pos[2] - 3.0).abs() < 1e-5);
    }

    /// `BuildOptions::body_props_raw` overrides the reconstructed material
    /// bytes at offsets 0x10-0x1F of the body_props payload. Mirrors pynifly's
    /// `body_props_raw` round-trip preservation (refs/io_scene_nifly/nif/
    /// collision.py:1318). The override must reach the writer's output —
    /// without it, callers re-exporting a vanilla mesh would lose damping,
    /// max-velocity, and Bethesda-specific flag bits.
    #[test]
    fn body_props_raw_overrides_reconstructed_material_bytes() {
        use crate::collision::polytope::build_fo4_polytope_collision;
        // Pynifly default sentinel: 00ff003f003fcd3e01024c3deeff7f7f
        let raw: [u8; 16] = [
            0x00, 0xff, 0x00, 0x3f, 0x00, 0x3f, 0xcd, 0x3e, 0x01, 0x02, 0x4c, 0x3d, 0xee, 0xff,
            0x7f, 0x7f,
        ];
        let opts = BuildOptions {
            body_props_raw: Some(raw),
            ..BuildOptions::default()
        };
        let blob = build_fo4_polytope_collision(&unit_cube_vertices(), &opts).unwrap();
        // Locate body_props in the materials array of the parsed PSD; the raw
        // override is supposed to land at offsets 0x10..0x20 of that 0x50 blob.
        // We re-read the raw bytes from the input blob (single-body builder
        // emits via from_tagxml so the writer has run; the bytes we want are
        // visible directly in the resulting packfile).
        // Walk the blob looking for the raw sentinel marker.
        let mut found = false;
        for window in blob.windows(16) {
            if window == raw {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "body_props_raw sentinel bytes not found in output blob — override silently dropped"
        );
    }

    /// `[Sphere, Polytope]` and `[CompressedMesh, Sphere]` have no vanilla
    /// precedent — must be rejected loudly with a clear message.
    #[test]
    fn sphere_mixed_with_other_bodies_is_rejected() {
        let verts = unit_cube_vertices();
        let err = build_fo4_multi_body_collision(
            &[
                MultiBodyShape::Sphere {
                    radius: 0.5,
                    position: [0.0, 0.0, 0.0],
                },
                MultiBodyShape::Polytope { vertices: verts },
            ],
            &BuildOptions::default(),
            None,
            None,
        )
        .expect_err("sphere + polytope must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Sphere") && msg.contains("sole body"),
            "expected rejection mentioning Sphere/sole-body, got: {msg}"
        );
    }

    fn irregular_cloud() -> Vec<[f32; 3]> {
        // Deterministic pseudo-random cloud (LCG) producing a richer hull with
        // many facets and coplanarity ties — exercises Quickhull paths a
        // symmetric cube does not.
        let mut s: u32 = 0x1234_5678;
        let mut next = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / 16_777_216.0) * 2.0 - 1.0
        };
        (0..40).map(|_| [next(), next(), next()]).collect()
    }

    #[test]
    fn build_large_compressed_mesh_is_byte_deterministic() {
        // >128 triangles forces multi-section splitting + AABB-tree node build,
        // a path the 12-triangle cube canary never reaches.
        let mut s: u32 = 0x9E37_79B9;
        let mut next = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / 16_777_216.0) * 10.0
        };
        let n_verts = 300usize;
        let verts: Vec<[f32; 3]> = (0..n_verts).map(|_| [next(), next(), next()]).collect();
        let tris: Vec<[u32; 3]> = (0..400)
            .map(|i| {
                let a = (i * 7) % n_verts;
                let b = (i * 13 + 1) % n_verts;
                let c = (i * 17 + 2) % n_verts;
                [a as u32, b as u32, c as u32]
            })
            .collect();
        let bodies = vec![MultiBodyShape::CompressedMesh {
            vertices: verts,
            triangles: tris,
        }];
        let metas = vec![BodyMeta::default()];
        let crcs: Vec<Option<u32>> = vec![Some(0xC0EB623D)];
        let opts = BuildOptions::default();
        let first = match build_fo4_multi_body_collision(&bodies, &opts, Some(&crcs), Some(&metas))
        {
            Ok(b) => b,
            Err(_) => return, // degenerate random mesh rejected — not the test's concern
        };
        for round in 0..32 {
            let again =
                build_fo4_multi_body_collision(&bodies, &opts, Some(&crcs), Some(&metas)).unwrap();
            assert_eq!(
                again, first,
                "round {round}: large CM not byte-deterministic"
            );
        }
    }

    fn boslp_thin_hull() -> Vec<[f32; 3]> {
        vec![
            [0.015272981f32, -0.1211541f32, 0.31871527f32],
            [0.015282315f32, -0.19177605f32, -0.18409109f32],
            [0.015289783f32, -0.16306832f32, 0.27304584f32],
            [0.015293516f32, -0.17751585f32, 0.22644706f32],
            [-0.015247883f32, 0.07752868f32, 0.3498696f32],
            [-0.015247883f32, 0.12695937f32, -0.31623152f32],
            [-0.015218013f32, 0.010676666f32, 0.3624818f32],
            [-0.015218013f32, 0.06500124f32, -0.3529618f32],
            [-0.015188141f32, -0.05638609f32, 0.35305583f32],
            [-0.015188141f32, -0.0018273537f32, -0.36247626f32],
            [-0.015160136f32, -0.06910085f32, -0.34986407f32],
            [-0.01515827f32, -0.121130675f32, 0.3187595f32],
            [-0.015139601f32, -0.16306832f32, 0.27304584f32],
            [-0.015133999f32, -0.13197218f32, -0.30985904f32],
            [-0.015130266f32, -0.17753927f32, 0.22649132f32],
            [-0.01511533f32, -0.17718804f32, -0.24892221f32],
            [-0.015111596f32, -0.19179948f32, -0.18409109f32],
            [0.015110556f32, 0.19179763f32, -0.19196817f32],
            [0.015116156f32, 0.17247961f32, -0.25693208f32],
            [0.015134826f32, 0.12698278f32, -0.31623152f32],
            [0.015134826f32, 0.17561732f32, 0.25946006f32],
            [0.015157229f32, 0.13602127f32, 0.31473246f32],
            [0.015160962f32, 0.06502466f32, -0.3529618f32],
            [0.015185233f32, 0.077552095f32, 0.3498696f32],
            [0.015190835f32, -0.0018273537f32, -0.36247626f32],
            [0.0152151035f32, 0.010676666f32, 0.3624818f32],
            [0.015220705f32, -0.06907744f32, -0.34986407f32],
            [0.015244977f32, -0.056315843f32, 0.3531001f32],
            [0.015250577f32, -0.13197218f32, -0.30985904f32],
            [0.015272981f32, -0.17718804f32, -0.24892221f32],
            [-0.015268419f32, 0.1724562f32, -0.25693208f32],
            [-0.0152721545f32, 0.13599785f32, 0.31473246f32],
            [-0.015279621f32, 0.1917742f32, -0.19196817f32],
            [-0.015290823f32, 0.17561732f32, 0.25946006f32],
            [-0.015290823f32, 0.17561732f32, 0.25946006f32],
            [-0.015290823f32, 0.17561732f32, 0.25946006f32],
        ]
    }

    /// Regression: a real FO76 thin-slab collision hull (boslpleftarmpart07.nif)
    /// whose near-coplanar vertices triggered nondeterministic Quickhull facet
    /// ordering the synthetic cube/irregular canaries did not reach.
    #[test]
    fn build_boslp_thin_hull_is_byte_deterministic() {
        let bodies = vec![MultiBodyShape::Polytope {
            vertices: boslp_thin_hull(),
        }];
        let metas = vec![BodyMeta {
            collision_filter_info: None,
            layer: 1,
            ..BodyMeta::default()
        }];
        let crcs: Vec<Option<u32>> = vec![Some(104858580)];
        let opts = BuildOptions {
            layer: 1,
            convex_radius: 0.01,
            ..BuildOptions::default()
        };
        let first = build_fo4_multi_body_collision(&bodies, &opts, Some(&crcs), Some(&metas))
            .expect("build");
        for round in 0..32 {
            let again =
                build_fo4_multi_body_collision(&bodies, &opts, Some(&crcs), Some(&metas)).unwrap();
            assert_eq!(
                again, first,
                "round {round}: boslp thin hull not byte-deterministic"
            );
        }
    }

    #[test]
    fn build_irregular_hull_is_byte_deterministic() {
        let bodies = vec![MultiBodyShape::Polytope {
            vertices: irregular_cloud(),
        }];
        let metas = vec![BodyMeta::default()];
        let crcs: Vec<Option<u32>> = vec![Some(0xC0EB623D)];
        let opts = BuildOptions::default();
        let first = build_fo4_multi_body_collision(&bodies, &opts, Some(&crcs), Some(&metas))
            .expect("build");
        for round in 0..32 {
            let again =
                build_fo4_multi_body_collision(&bodies, &opts, Some(&crcs), Some(&metas)).unwrap();
            assert_eq!(
                again, first,
                "round {round}: irregular hull not byte-deterministic"
            );
        }
    }

    /// Identical build requests must produce identical bytes. A cube hull has
    /// coplanar triangle pairs per face, exercising hull::extract_boundary_loop
    /// where the HashMap-start nondeterminism lived.
    #[test]
    fn build_is_byte_deterministic_across_repeated_builds() {
        let cases: Vec<Vec<MultiBodyShape>> = vec![
            // single static polytope (boundary-loop path)
            vec![MultiBodyShape::Polytope {
                vertices: unit_cube_vertices(),
            }],
            // single compressed mesh
            vec![MultiBodyShape::CompressedMesh {
                vertices: unit_cube_vertices(),
                triangles: unit_cube_triangles(),
            }],
            // compound of two polytopes
            vec![MultiBodyShape::Compound {
                children: vec![
                    CompoundChild {
                        transform: CompoundChild::identity_transform(),
                        kind: CompoundChildKind::Polytope {
                            vertices: unit_cube_vertices(),
                        },
                    },
                    CompoundChild {
                        transform: CompoundChild::identity_transform(),
                        kind: CompoundChildKind::Polytope {
                            vertices: unit_cube_vertices(),
                        },
                    },
                ],
            }],
            // mixed CM + polytope with a keyframed body (Safe01 pattern)
            vec![
                MultiBodyShape::CompressedMesh {
                    vertices: unit_cube_vertices(),
                    triangles: unit_cube_triangles(),
                },
                MultiBodyShape::Polytope {
                    vertices: unit_cube_vertices(),
                },
            ],
        ];
        for (case_index, bodies) in cases.iter().enumerate() {
            let metas: Vec<BodyMeta> = bodies
                .iter()
                .enumerate()
                .map(|(i, _)| BodyMeta {
                    collision_filter_info: None,
                    // case 3: second body keyframed (ANIMSTATIC) to cover the
                    // has_keyframed motion-cinfo branch
                    motion_type: if case_index == 3 && i == 1 {
                        BodyMotionType::Keyframed
                    } else {
                        BodyMotionType::Static
                    },
                    ..BodyMeta::default()
                })
                .collect();
            let material_crcs: Vec<Option<u32>> = bodies.iter().map(|_| Some(0xC0EB623D)).collect();
            let opts = BuildOptions::default();
            let first =
                build_fo4_multi_body_collision(bodies, &opts, Some(&material_crcs), Some(&metas))
                    .unwrap_or_else(|e| panic!("case {case_index}: {e}"));
            for round in 0..16 {
                let again = build_fo4_multi_body_collision(
                    bodies,
                    &opts,
                    Some(&material_crcs),
                    Some(&metas),
                )
                .unwrap();
                assert_eq!(
                    again, first,
                    "case {case_index} round {round}: build output not byte-deterministic"
                );
            }
        }
    }

    fn typed_member_f32(value: &HkxValue, name: &str) -> Option<f32> {
        let HkxValue::TypedObject { members, .. } = value else {
            return None;
        };
        members
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| match m.value {
                HkxValue::F32(v) => Some(v),
                _ => None,
            })
    }

    fn thin_panel_havok_vertices() -> Vec<[f32; 3]> {
        vec![
            [-0.5, -0.5, -0.0002],
            [0.5, -0.5, -0.0002],
            [-0.5, 0.5, -0.0002],
            [0.5, 0.5, -0.0002],
            [-0.5, -0.5, 0.0002],
            [0.5, -0.5, 0.0002],
            [-0.5, 0.5, 0.0002],
            [0.5, 0.5, 0.0002],
        ]
    }

    #[test]
    fn dynamic_clutter_mass_factor_is_bounded() {
        let body = MultiBodyShape::Polytope {
            vertices: thin_panel_havok_vertices(),
        };
        let cinfo =
            build_dynamic_clutter_motion_cinfo(&body, [0.0; 4], [0.0, 0.0, 0.0, 1.0], None, None);
        let mf = typed_member_f32(&cinfo, "massFactor").expect("massFactor present");
        assert!(
            mf <= 100.0 + 1e-3,
            "massFactor must be clamped to <= ~100, got {mf}"
        );
    }
}
