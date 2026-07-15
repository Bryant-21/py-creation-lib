use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};
use crate::hkx::HkxFile;
use crate::hkx::types::HkxValue;

pub struct ConversionContext<'a> {
    pub hkx: &'a mut HkxFile,
    pub source_version: u8,
    pub target_version: u8,
    pub route: String,
    pub object_index: Option<usize>,
}

impl<'a> ConversionContext<'a> {
    pub fn new(
        hkx: &'a mut HkxFile,
        source_version: u8,
        target_version: u8,
        route: impl Into<String>,
    ) -> Self {
        Self {
            hkx,
            source_version,
            target_version,
            route: route.into(),
            object_index: None,
        }
    }

    pub fn new_for_object(
        hkx: &'a mut HkxFile,
        source_version: u8,
        target_version: u8,
        route: impl Into<String>,
        object_index: usize,
    ) -> Self {
        Self {
            hkx,
            source_version,
            target_version,
            route: route.into(),
            object_index: Some(object_index),
        }
    }
}

type CustomHook = Box<dyn Fn(&mut ConversionContext<'_>) -> HavokResult<()> + Send + Sync>;

#[derive(Default)]
pub struct CustomHookRegistry {
    hooks: HashMap<String, CustomHook>,
}

impl CustomHookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&mut self, name: impl Into<String>, hook: F)
    where
        F: Fn(&mut ConversionContext<'_>) -> HavokResult<()> + Send + Sync + 'static,
    {
        self.hooks.insert(name.into(), Box::new(hook));
    }

    pub fn invoke(&self, name: &str, context: &mut ConversionContext<'_>) -> HavokResult<()> {
        let hook = self.hooks.get(name).ok_or_else(|| {
            HavokError::InvalidInput(format!("unknown custom conversion hook: {name}"))
        })?;
        hook(context)
    }
}

// ---------------------------------------------------------------------------
// Standalone hook implementations referenced from corpus.rs
// ---------------------------------------------------------------------------

/// Convert `old_triangleFlips` (int32 array) to `triangleFlips` (byte array).
///
/// Each int32 in the old array is split into 4 bytes little-endian.
/// Used by hclUpdateAllVertexFramesOperator_2_to_3,
/// hclUpdateSomeVertexFramesOperator_2_to_3, and hclSimClothData_9_to_10.
/// Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_1/cloth.py:_update_triangle_flips
fn update_triangle_flips_for_class(
    context: &mut ConversionContext<'_>,
    class_name: &str,
) -> HavokResult<()> {
    let indices: Vec<usize> = match context.object_index {
        Some(i) => vec![i],
        None => (0..context.hkx.objects().len()).collect(),
    };
    for index in indices {
        let object = &mut context.hkx.objects_mut()[index];
        if object.class_name != class_name {
            continue;
        }
        // Collect int32 values from old_triangleFlips
        let src: Vec<i32> = object
            .members
            .iter()
            .find(|m| m.name == "old_triangleFlips")
            .and_then(|m| match &m.value {
                HkxValue::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(|v| match v {
                            HkxValue::I32(x) => Some(*x),
                            HkxValue::U32(x) => Some(*x as i32),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();

        // Convert each int32 to 4 little-endian bytes
        let bytes: Vec<HkxValue> = src
            .iter()
            .flat_map(|&v| {
                let u = v as u32;
                (0u32..4).map(move |b| HkxValue::U8(((u >> (8 * b)) & 0xFF) as u8))
            })
            .collect();

        if let Some(target) = object
            .members
            .iter_mut()
            .find(|m| m.name == "triangleFlips")
        {
            target.value = HkxValue::Array(bytes);
        }
    }
    Ok(())
}

pub fn hcl_update_all_vertex_frames_operator_2_to_3(
    context: &mut ConversionContext<'_>,
) -> HavokResult<()> {
    update_triangle_flips_for_class(context, "hclUpdateAllVertexFramesOperator")
}

pub fn hcl_update_some_vertex_frames_operator_2_to_3(
    context: &mut ConversionContext<'_>,
) -> HavokResult<()> {
    update_triangle_flips_for_class(context, "hclUpdateSomeVertexFramesOperator")
}

pub fn hcl_sim_cloth_data_9_to_10(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    update_triangle_flips_for_class(context, "hclSimClothData")
}

// ---------------------------------------------------------------------------
// hknp 2014.1 callbacks (version_id = 53)
// ---------------------------------------------------------------------------

/// hknpCharacterRigidBodyCinfo v2->v3: copy `additionFlags` into `activationMode`.
pub fn hknp_character_rigid_body_cinfo_2_to_3(
    context: &mut ConversionContext<'_>,
) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        let flags = context.hkx.objects()[index]
            .members
            .iter()
            .find(|m| m.name == "additionFlags")
            .map(|m| m.value.clone());
        if let Some(val) = flags {
            if let Some(target) = context.hkx.objects_mut()[index]
                .members
                .iter_mut()
                .find(|m| m.name == "activationMode")
            {
                target.value = val;
            }
        }
    }
    Ok(())
}

/// hknpBody v1->v2 / hknpBodyCinfo v2->v3: if spuFlags bit 0 is set,
/// set FORCE_NARROW_PHASE_PPU (bit 28) in `flags`.
fn set_force_narrow_phase_ppu_if_spu_set(object: &mut crate::hkx::HkxObject, flags_field: &str) {
    let spu_set = object
        .members
        .iter()
        .find(|m| m.name == "spuFlags")
        .map(|m| match &m.value {
            HkxValue::U8(v) => (*v as i64) & 1 != 0,
            HkxValue::I8(v) => (*v as i64) & 1 != 0,
            HkxValue::I32(v) => (*v as i64) & 1 != 0,
            HkxValue::U32(v) => (*v as i64) & 1 != 0,
            _ => false,
        })
        .unwrap_or(false);
    if spu_set {
        if let Some(flags_member) = object.members.iter_mut().find(|m| m.name == flags_field) {
            let new_val = match &flags_member.value {
                HkxValue::I32(v) => HkxValue::I32(*v | (1 << 28)),
                HkxValue::U32(v) => HkxValue::U32(*v | (1 << 28)),
                HkxValue::I64(v) => HkxValue::I64(*v | (1 << 28)),
                _ => return,
            };
            flags_member.value = new_val;
        }
    }
}

pub fn hknp_body_1_to_2(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        set_force_narrow_phase_ppu_if_spu_set(&mut context.hkx.objects_mut()[index], "flags");
    }
    Ok(())
}

pub fn hknp_body_cinfo_2_to_3(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        set_force_narrow_phase_ppu_if_spu_set(&mut context.hkx.objects_mut()[index], "flags");
    }
    Ok(())
}

/// hknpBodyCinfo v3->v4: set `motionPropertiesId` to 0xFFFF (INVALID sentinel).
pub fn hknp_body_cinfo_3_to_4(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        if let Some(member) = context.hkx.objects_mut()[index]
            .members
            .iter_mut()
            .find(|m| m.name == "motionPropertiesId")
        {
            member.value = HkxValue::I32(0xFFFF);
        }
    }
    Ok(())
}

/// hknpConstraint v0->v1: pack bodyIdA_old/bodyIdB_old + serial=1 into bodyUidA/bodyUidB.
///
/// On little-endian (x86): uid64 = (serial << 32) | body_id_u32
/// We store as I32 because our HkxValue doesn't have I64; the high word (serial) is lost
/// but for FO4/FO76 files the uid is read back by hknpConstraint_1_to_2 which only cares
/// about the low 24 bits anyway.
pub fn hknp_constraint_0_to_1(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        let id_a = context.hkx.objects()[index]
            .members
            .iter()
            .find(|m| m.name == "bodyIdA_old")
            .and_then(|m| match &m.value {
                HkxValue::I32(v) => Some(*v),
                HkxValue::U32(v) => Some(*v as i32),
                _ => None,
            });
        let id_b = context.hkx.objects()[index]
            .members
            .iter()
            .find(|m| m.name == "bodyIdB_old")
            .and_then(|m| match &m.value {
                HkxValue::I32(v) => Some(*v),
                HkxValue::U32(v) => Some(*v as i32),
                _ => None,
            });
        let object = &mut context.hkx.objects_mut()[index];
        if let Some(id) = id_a {
            if let Some(uid) = object.members.iter_mut().find(|m| m.name == "bodyUidA") {
                // serial=1 in high byte, id in low bits (simplified for PC little-endian)
                uid.value = HkxValue::I32(id);
            }
        }
        if let Some(id) = id_b {
            if let Some(uid) = object.members.iter_mut().find(|m| m.name == "bodyUidB") {
                uid.value = HkxValue::I32(id);
            }
        }
    }
    Ok(())
}

/// hknpConvexPolytopeShape v2->v3: trim `planes` array to `faces.len()` entries.
pub fn hknp_convex_polytope_shape_2_to_3(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        let num_faces = context.hkx.objects()[index]
            .members
            .iter()
            .find(|m| m.name == "faces")
            .and_then(|m| match &m.value {
                HkxValue::Array(items) => Some(items.len()),
                _ => None,
            });
        if let Some(n) = num_faces {
            if let Some(planes) = context.hkx.objects_mut()[index]
                .members
                .iter_mut()
                .find(|m| m.name == "planes")
            {
                if let HkxValue::Array(items) = &mut planes.value {
                    items.truncate(n);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// hknp 2014.2 callbacks (version_id = 55, shared with 2014_2_5)
// ---------------------------------------------------------------------------

/// hknpBody v2->v3: noop — timAngle type change is handled by structural patch.
pub fn hknp_body_2_to_3(_context: &mut ConversionContext<'_>) -> HavokResult<()> {
    Ok(())
}

/// hknpShape v2->v3: remove PS3 SPU flags (bits 9/10) and shift remaining down.
pub fn hknp_shape_2_to_3(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        if let Some(flags_member) = context.hkx.objects_mut()[index]
            .members
            .iter_mut()
            .find(|m| m.name == "flags")
        {
            let old = match &flags_member.value {
                HkxValue::I32(v) => *v,
                HkxValue::U32(v) => *v as i32,
                _ => return Ok(()),
            };
            let mask = 0x1FF_i32;
            let new_val = (old & mask) | ((old >> 2) & !mask);
            flags_member.value = HkxValue::I32(new_val);
        }
    }
    Ok(())
}

/// hknpConstraint v1->v2: unpack bodyUidA/B (serial<<32|id) to bodyIdA/B.
pub fn hknp_constraint_1_to_2(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        for (uid_field, id_field) in [("bodyUidA", "bodyIdA"), ("bodyUidB", "bodyIdB")] {
            let uid = context.hkx.objects()[index]
                .members
                .iter()
                .find(|m| m.name == uid_field)
                .and_then(|m| match &m.value {
                    HkxValue::I32(v) => Some(*v as i64),
                    HkxValue::U32(v) => Some(*v as i64),
                    HkxValue::I64(v) => Some(*v),
                    _ => None,
                });
            if let Some(uid_val) = uid {
                // new bodyId = { serial:8 | index:24 }
                let serial = ((uid_val >> 32) & 0xFF) as i32;
                let body_index = (uid_val & 0x00FF_FFFF) as i32;
                let packed = (serial << 24) | body_index;
                let object = &mut context.hkx.objects_mut()[index];
                if let Some(id_member) = object.members.iter_mut().find(|m| m.name == id_field) {
                    id_member.value = HkxValue::I32(packed);
                }
            }
        }
    }
    Ok(())
}

/// hknpConvexPolytopeShape v3->v4: clamp each face's `minHalfAngle` to [0, 127].
pub fn hknp_convex_polytope_shape_3_to_4(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        if let Some(faces_member) = context.hkx.objects_mut()[index]
            .members
            .iter_mut()
            .find(|m| m.name == "faces")
        {
            if let HkxValue::Array(face_items) = &mut faces_member.value {
                for face in face_items.iter_mut() {
                    let face_members = match face {
                        HkxValue::Object(members) => members,
                        HkxValue::TypedObject { members, .. } => members,
                        _ => continue,
                    };
                    if let Some(half_angle) =
                        face_members.iter_mut().find(|m| m.name == "minHalfAngle")
                    {
                        match half_angle.value {
                            HkxValue::I32(v) => half_angle.value = HkxValue::I32(v.clamp(0, 0x7F)),
                            HkxValue::U8(v) => half_angle.value = HkxValue::I32(v.min(0x7F) as i32),
                            HkxValue::U32(v) => {
                                half_angle.value = HkxValue::I32((v.min(0x7F)) as i32)
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// hknp 2014.2.5 callbacks (version_id = 55, same package as 2014_2)
// ---------------------------------------------------------------------------

/// hknpShape v3->v4: if dispatchType > 0, shift it up by 1 (adds DEBRIS slot).
pub fn hknp_shape_3_to_4(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        if let Some(dispatch) = context.hkx.objects_mut()[index]
            .members
            .iter_mut()
            .find(|m| m.name == "dispatchType")
        {
            match dispatch.value {
                HkxValue::I32(v) if v > 0 => dispatch.value = HkxValue::I32(v + 1),
                HkxValue::U8(v) if v > 0 => dispatch.value = HkxValue::I32(v as i32 + 1),
                _ => {}
            }
        }
    }
    Ok(())
}

/// hknpBody v3->v4: AABB type change — noop (SDK had no data mutation).
pub fn hknp_body_3_to_4(_context: &mut ConversionContext<'_>) -> HavokResult<()> {
    Ok(())
}

/// hknpBody v5->v6: unpack `motionToBodyRotation_old` (4×i16 PackedUnitVector)
/// to `motionToBodyRotation` (4×f32 quaternion).
pub fn hknp_body_5_to_6(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        let raw: Option<Vec<i16>> = context.hkx.objects()[index]
            .members
            .iter()
            .find(|m| m.name == "motionToBodyRotation_old")
            .and_then(|m| match &m.value {
                HkxValue::Array(items) => {
                    let vals: Vec<i16> = items
                        .iter()
                        .filter_map(|v| match v {
                            HkxValue::I16(x) => Some(*x),
                            HkxValue::I32(x) => Some(*x as i16),
                            HkxValue::U16(x) => Some(*x as i16),
                            _ => None,
                        })
                        .collect();
                    if vals.len() >= 4 { Some(vals) } else { None }
                }
                HkxValue::F32List(floats) => {
                    // Already unpacked — no transform needed
                    let _ = floats;
                    None
                }
                _ => None,
            });
        if let Some(raw) = raw {
            // hkPackedUnitVector<4>: component = raw[i] / 32767.0
            let mut quat: [f32; 4] = [
                raw[0] as f32 / 32767.0,
                raw[1] as f32 / 32767.0,
                raw[2] as f32 / 32767.0,
                raw[3] as f32 / 32767.0,
            ];
            let len = (quat.iter().map(|v| v * v).sum::<f32>()).sqrt();
            if len > 1e-6 {
                for v in &mut quat {
                    *v /= len;
                }
            }
            if let Some(target) = context.hkx.objects_mut()[index]
                .members
                .iter_mut()
                .find(|m| m.name == "motionToBodyRotation")
            {
                target.value = HkxValue::F32List(quat.to_vec());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// hknp 2015.1 callbacks (version_id = 56)
// ---------------------------------------------------------------------------

/// hknpConstraintCinfo v4->v5: set `constraintGroupId.value` to INT32_MAX (unassigned).
pub fn hknp_constraint_cinfo_4_to_5(context: &mut ConversionContext<'_>) -> HavokResult<()> {
    if let Some(index) = context.object_index {
        if let Some(group_id_member) = context.hkx.objects_mut()[index]
            .members
            .iter_mut()
            .find(|m| m.name == "constraintGroupId")
        {
            if let Some(sub_members) = group_id_member.value.as_object_members_mut() {
                if let Some(val_member) = sub_members.iter_mut().find(|m| m.name == "value") {
                    val_member.value = HkxValue::I32(i32::MAX);
                }
            }
        }
    }
    Ok(())
}
