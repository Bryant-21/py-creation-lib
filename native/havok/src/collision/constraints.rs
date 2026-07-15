//! Lift a FO76 `bhkPhysicsSystem`'s constraint sub-graph into a self-contained
//! form the FO4 multi-body re-encode can graft back on.
//!
//! The NIF collision re-encode (`nif_core::fo76_collision` → [`super::multi_body`])
//! rebuilds each body's shape from scratch and has no notion of constraints, so an
//! articulated assembly (a hanging chime, swinging sign, …) loses the ragdoll
//! constraints that link its dynamic bodies — it stops moving and its segments get
//! reclassified as loose clutter. FO76 and FO4 use the *same* `hkpRagdollConstraintData`
//! class here (vanilla `TrapCanChimes01` is byte-for-byte the same shape), so the
//! fix is to carry the constraint objects + `constraintCinfos` through verbatim.

use std::collections::{HashSet, VecDeque};

use crate::error::HavokResult;
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

/// A constraint sub-graph lifted from a source physics blob, ready to graft onto
/// a rebuilt FO4 physics system. All pointers inside [`GraftedConstraints::objects`]
/// are already local to that vec.
#[derive(Debug, Clone, Default)]
pub struct GraftedConstraints {
    /// The constraint objects (`hkpRagdollConstraintData` + motors) with every
    /// internal pointer remapped to be local to this vec.
    pub objects: Vec<HkxObject>,
    /// One entry per source constraint.
    pub cinfos: Vec<GraftCinfo>,
}

/// One `hknpConstraintCinfo`, decoded from the source and pending body-index remap.
#[derive(Debug, Clone, Copy)]
pub struct GraftCinfo {
    /// Bodies the constraint links, as **source** physics-system body indices.
    /// The NIF caller remaps these to the rebuilt output body indices.
    pub body_a: u32,
    pub body_b: u32,
    /// Index into [`GraftedConstraints::objects`] of this constraint's data object.
    pub data_object: usize,
    pub flags: u16,
}

impl GraftedConstraints {
    pub fn is_empty(&self) -> bool {
        self.cinfos.is_empty()
    }
}

/// Lift the constraint sub-graph out of a FO76 `bhkPhysicsSystem` blob. Returns
/// `Ok(None)` when the blob parses but carries no constraints. Body handles come
/// out as source body indices (see [`GraftCinfo::body_a`]); empty ragdoll-motor
/// arrays are padded to the three null slots vanilla ships (see
/// [`normalize_ragdoll_motor_slots`]).
pub fn extract_grafted_constraints(blob: &[u8]) -> HavokResult<Option<GraftedConstraints>> {
    let hkx = HkxFile::read(blob)?;
    let objects = hkx.objects();

    let Some(psd) = objects
        .iter()
        .find(|obj| obj.class_name == "hknpPhysicsSystemData")
    else {
        return Ok(None);
    };
    let Some(HkxValue::Array(entries)) = psd
        .members
        .iter()
        .find(|member| member.name == "constraintCinfos")
        .map(|member| &member.value)
    else {
        return Ok(None);
    };
    if entries.is_empty() {
        return Ok(None);
    }

    struct SrcCinfo {
        body_a: u32,
        body_b: u32,
        data_src: usize,
        flags: u16,
    }
    let mut src_cinfos: Vec<SrcCinfo> = Vec::new();
    for entry in entries {
        let Some(members) = entry.as_object_members() else {
            continue;
        };
        let (Some(body_a), Some(body_b), Some(data_src)) = (
            body_handle(members, "bodyA"),
            body_handle(members, "bodyB"),
            members
                .iter()
                .find(|member| member.name == "constraintData")
                .and_then(|member| match member.value {
                    HkxValue::Pointer(Some(index)) => Some(index),
                    _ => None,
                }),
        ) else {
            continue;
        };
        let flags = members
            .iter()
            .find(|member| member.name == "flags")
            .and_then(|member| extract_u32(&member.value))
            .unwrap_or(0) as u16;
        src_cinfos.push(SrcCinfo {
            body_a,
            body_b,
            data_src,
            flags,
        });
    }
    if src_cinfos.is_empty() {
        return Ok(None);
    }

    // Collect every object reachable from a constraintData pointer.
    let mut visited: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<usize> = src_cinfos.iter().map(|cinfo| cinfo.data_src).collect();
    while let Some(index) = queue.pop_front() {
        if index >= objects.len() || !visited.insert(index) {
            continue;
        }
        let mut targets = Vec::new();
        for member in &objects[index].members {
            collect_pointer_targets(&member.value, &mut targets);
        }
        for target in targets {
            if !visited.contains(&target) {
                queue.push_back(target);
            }
        }
    }
    let mut collected: Vec<usize> = visited.into_iter().collect();
    collected.sort_unstable();

    // source index → local index for the lifted sub-graph.
    let mut remap = vec![None; objects.len()];
    for (local, &src) in collected.iter().enumerate() {
        remap[src] = Some(local);
    }
    let mut local_objects: Vec<HkxObject> =
        collected.iter().map(|&src| objects[src].clone()).collect();
    for object in &mut local_objects {
        for member in &mut object.members {
            remap_pointers(&mut member.value, &remap);
        }
    }

    normalize_ragdoll_motor_slots(&mut local_objects);

    let cinfos = src_cinfos
        .iter()
        .map(|cinfo| GraftCinfo {
            body_a: cinfo.body_a,
            body_b: cinfo.body_b,
            data_object: remap[cinfo.data_src].expect("constraintData reached by BFS"),
            flags: cinfo.flags,
        })
        .collect();

    Ok(Some(GraftedConstraints {
        objects: local_objects,
        cinfos,
    }))
}

/// Read an `hknpBodyId` handle: either the flattened integer or the source's
/// `{ serialAndIndex }` wrapper. The value is the physics-system body index.
fn body_handle(members: &[HkxMember], name: &str) -> Option<u32> {
    let member = members.iter().find(|member| member.name == name)?;
    if let Some(inner) = member.value.as_object_members() {
        inner
            .iter()
            .find(|inner| inner.name == "serialAndIndex")
            .and_then(|inner| extract_u32(&inner.value))
    } else {
        extract_u32(&member.value)
    }
}

fn extract_u32(value: &HkxValue) -> Option<u32> {
    match value {
        HkxValue::U8(v) => Some(*v as u32),
        HkxValue::U16(v) => Some(*v as u32),
        HkxValue::U32(v) => Some(*v),
        HkxValue::U64(v) => Some(*v as u32),
        HkxValue::I8(v) => Some(*v as u32),
        HkxValue::I16(v) => Some(*v as u32),
        HkxValue::I32(v) => Some(*v as u32),
        HkxValue::I64(v) => Some(*v as u32),
        _ => None,
    }
}

fn collect_pointer_targets(value: &HkxValue, out: &mut Vec<usize>) {
    match value {
        HkxValue::Pointer(Some(index)) => out.push(*index),
        HkxValue::Array(values) => values
            .iter()
            .for_each(|value| collect_pointer_targets(value, out)),
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => members
            .iter()
            .for_each(|member| collect_pointer_targets(&member.value, out)),
        _ => {}
    }
}

fn remap_pointers(value: &mut HkxValue, remap: &[Option<usize>]) {
    match value {
        HkxValue::Pointer(Some(index)) => {
            *value = HkxValue::Pointer(remap.get(*index).copied().flatten());
        }
        HkxValue::Array(values) => values
            .iter_mut()
            .for_each(|value| remap_pointers(value, remap)),
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => members
            .iter_mut()
            .for_each(|member| remap_pointers(&mut member.value, remap)),
        _ => {}
    }
}

/// FO4 reads three `hkpRagdollConstraintData.atoms.ragdollMotors.motors` slots
/// (twist / plane / cone) on activation. FO76 trap chimes ship an empty array,
/// which FO4 indexes out of bounds. Vanilla FO4 `TrapCanChimes01` ships three
/// **null** pointers there and free-swings correctly, so pad every un-populated
/// ragdoll to three nulls — matching vanilla exactly. A ragdoll that already
/// carries real motors is left untouched.
fn normalize_ragdoll_motor_slots(objects: &mut [HkxObject]) {
    for object in objects.iter_mut() {
        if object.class_name != "hkpRagdollConstraintData" || ragdoll_motors_populated(object) {
            continue;
        }
        for member in &mut object.members {
            if member.name != "atoms" {
                continue;
            }
            let Some(atoms) = member.value.as_object_members_mut() else {
                continue;
            };
            for atom in atoms.iter_mut() {
                if atom.name != "ragdollMotors" {
                    continue;
                }
                let Some(ragdoll_motors) = atom.value.as_object_members_mut() else {
                    continue;
                };
                for field in ragdoll_motors.iter_mut() {
                    if field.name == "motors" {
                        field.value = HkxValue::Array(vec![
                            HkxValue::Pointer(None),
                            HkxValue::Pointer(None),
                            HkxValue::Pointer(None),
                        ]);
                    }
                }
            }
            break;
        }
    }
}

fn ragdoll_motors_populated(object: &HkxObject) -> bool {
    object.members.iter().any(|member| {
        member.name == "atoms"
            && member
                .value
                .as_object_members()
                .map(|atoms| {
                    atoms.iter().any(|atom| {
                        atom.name == "ragdollMotors"
                            && atom
                                .value
                                .as_object_members()
                                .map(|fields| {
                                    fields.iter().any(|field| {
                                        field.name == "motors"
                                            && motors_pointer_populated(&field.value)
                                    })
                                })
                                .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
    })
}

fn motors_pointer_populated(value: &HkxValue) -> bool {
    match value {
        HkxValue::Pointer(Some(_)) => true,
        HkxValue::Array(values) => values
            .iter()
            .any(|value| matches!(value, HkxValue::Pointer(Some(_)))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ragdoll_with_motors(motors: HkxValue) -> HkxObject {
        HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hkpRagdollConstraintData".to_string(),
            members: vec![HkxMember {
                name: "atoms".to_string(),
                value: HkxValue::Object(vec![HkxMember {
                    name: "ragdollMotors".to_string(),
                    value: HkxValue::Object(vec![HkxMember {
                        name: "motors".to_string(),
                        value: motors,
                    }]),
                }]),
            }],
        }
    }

    fn motors_of(object: &HkxObject) -> Vec<HkxValue> {
        let atoms = object.members[0].value.as_object_members().unwrap();
        let ragdoll = atoms[0].value.as_object_members().unwrap();
        match &ragdoll[0].value {
            HkxValue::Array(values) => values.clone(),
            _ => panic!("motors not an array"),
        }
    }

    #[test]
    fn empty_motors_pad_to_three_nulls_like_vanilla() {
        // FO76 trap chimes ship a zero-length motors array; vanilla FO4 ships three
        // null pointers. We must pad, not inject a real motor (that would drive the
        // joints instead of letting them free-swing).
        let mut objects = vec![ragdoll_with_motors(HkxValue::Array(Vec::new()))];
        normalize_ragdoll_motor_slots(&mut objects);
        assert_eq!(objects.len(), 1, "must not append a motor object");
        assert_eq!(
            motors_of(&objects[0]),
            vec![
                HkxValue::Pointer(None),
                HkxValue::Pointer(None),
                HkxValue::Pointer(None)
            ]
        );
    }

    #[test]
    fn populated_motors_are_left_untouched() {
        let motors = HkxValue::Array(vec![
            HkxValue::Pointer(Some(7)),
            HkxValue::Pointer(Some(7)),
            HkxValue::Pointer(Some(7)),
        ]);
        let mut objects = vec![ragdoll_with_motors(motors.clone())];
        normalize_ragdoll_motor_slots(&mut objects);
        assert_eq!(
            motors_of(&objects[0]),
            vec![
                HkxValue::Pointer(Some(7)),
                HkxValue::Pointer(Some(7)),
                HkxValue::Pointer(Some(7)),
            ]
        );
    }
}
