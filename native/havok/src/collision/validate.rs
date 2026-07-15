//! Report-only collision validation: checks converted FO4 collision bodies
//! against vanilla-derived invariants. Pure — never panics, never blocks.

use serde::{Deserialize, Serialize};

use crate::hkx::model::{HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

#[derive(Debug, Clone, Deserialize)]
pub struct Invariants {
    #[serde(default)]
    pub layers: Vec<u32>,
    #[serde(default)]
    pub flags: Vec<i64>,
    #[serde(default)]
    pub quality_ids: Vec<i64>,
    #[serde(default)]
    pub known_body_shape_classes: Vec<String>,
    #[serde(default)]
    pub shape_tag_codec_info: Option<u64>,
    #[serde(default)]
    pub convex_radius_min: Option<f32>,
    #[serde(default)]
    pub convex_radius_max: Option<f32>,
    #[serde(default = "default_dynamic_flag")]
    pub dynamic_flag_bit: i64,
    #[serde(default = "default_invalid_motion_id")]
    pub invalid_motion_id: i64,
    #[serde(default = "default_min_hull_vertices")]
    pub min_hull_vertices: usize,
    #[serde(default = "default_degenerate_extent_eps")]
    pub degenerate_extent_eps: f32,
    #[serde(default = "default_thin_hull_ratio")]
    pub thin_hull_ratio: f32,
}

fn default_dynamic_flag() -> i64 {
    128
}
fn default_invalid_motion_id() -> i64 {
    0x7FFF_FFFF
}
fn default_min_hull_vertices() -> usize {
    4
}
fn default_degenerate_extent_eps() -> f32 {
    1e-4
}
fn default_thin_hull_ratio() -> f32 {
    0.01
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule_id: String,
    pub severity: Severity,
    pub body_index: Option<usize>,
    pub shape_index: Option<usize>,
    pub observed: String,
    pub expected: String,
    pub message: String,
}

fn member<'a>(obj: &'a HkxObject, name: &str) -> Option<&'a HkxValue> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .map(|m| &m.value)
}

fn object_members(value: &HkxValue) -> Option<&[HkxMember]> {
    match value {
        HkxValue::Object(m) | HkxValue::TypedObject { members: m, .. } => Some(m),
        _ => None,
    }
}

fn members_get<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a HkxValue> {
    members.iter().find(|m| m.name == name).map(|m| &m.value)
}

fn array(value: &HkxValue) -> Option<&[HkxValue]> {
    match value {
        HkxValue::Array(items) => Some(items),
        _ => None,
    }
}

fn as_i64(value: &HkxValue) -> Option<i64> {
    match value {
        HkxValue::Bool(b) => Some(*b as i64),
        HkxValue::I8(x) => Some(*x as i64),
        HkxValue::U8(x) => Some(*x as i64),
        HkxValue::I16(x) => Some(*x as i64),
        HkxValue::U16(x) => Some(*x as i64),
        HkxValue::I32(x) => Some(*x as i64),
        HkxValue::U32(x) => Some(*x as i64),
        HkxValue::I64(x) => Some(*x),
        HkxValue::U64(x) => i64::try_from(*x).ok(),
        _ => None,
    }
}

fn as_f32(value: &HkxValue) -> Option<f32> {
    match value {
        HkxValue::F32(x) | HkxValue::Half(x) => Some(*x),
        _ => None,
    }
}

pub fn validate_objects(objects: &[HkxObject], inv: &Invariants) -> Vec<Violation> {
    let mut out = Vec::new();

    let psd = match objects
        .iter()
        .find(|o| o.class_name == "hknpPhysicsSystemData" || o.class_name == "hknpRagdollData")
    {
        Some(p) => p,
        None => {
            out.push(Violation {
                rule_id: "missing_physics_system_data".into(),
                severity: Severity::Error,
                body_index: None,
                shape_index: None,
                observed: "none".into(),
                expected: "hknpPhysicsSystemData or hknpRagdollData".into(),
                message: "blob has no hknpPhysicsSystemData/hknpRagdollData object".into(),
            });
            return out;
        }
    };

    let body_cinfos = member(psd, "bodyCinfos").and_then(array).unwrap_or(&[]);
    let motion_cinfos = member(psd, "motionCinfos").and_then(array).unwrap_or(&[]);
    let motion_properties = member(psd, "motionProperties")
        .and_then(array)
        .unwrap_or(&[]);

    check_structural(
        psd,
        body_cinfos,
        motion_cinfos,
        motion_properties,
        objects,
        &mut out,
    );
    check_motion_finiteness(motion_cinfos, &mut out);

    for (i, body) in body_cinfos.iter().enumerate() {
        let members = match object_members(body) {
            Some(m) => m,
            None => continue,
        };
        check_body_semantic(members, i, motion_cinfos, inv, &mut out);
        check_body_finiteness(members, i, &mut out);
        check_body_geometry(objects, members, i, inv, &mut out);
    }

    out
}

/// Mass / inertia / center-of-mass live on the MOTION cinfo, not the body cinfo
/// (verified against vanilla FO4: body cinfos carry no `mass` member; the producer
/// writes `inverseMass`/`inverseInertiaLocal`/`centerOfMassWorld` on the motion
/// cinfo — see `multi_body.rs::build_keyframed_motion_cinfo`). A NaN/inf here is a
/// prime runtime-NaN/CTD source, so each is an ERROR.
fn check_motion_finiteness(motions: &[HkxValue], out: &mut Vec<Violation>) {
    for (i, motion) in motions.iter().enumerate() {
        let members = match object_members(motion) {
            Some(m) => m,
            None => continue,
        };
        if let Some(inv_mass) = members_get(members, "inverseMass").and_then(as_f32) {
            if !inv_mass.is_finite() {
                out.push(Violation {
                    rule_id: "non_finite_motion_mass".into(),
                    severity: Severity::Error,
                    body_index: None,
                    shape_index: None,
                    observed: format!("inverseMass={inv_mass}"),
                    expected: "finite".into(),
                    message: format!("motionCinfo {i} inverseMass is NaN/inf"),
                });
            }
        }
        for (field, rule, label) in [
            (
                "inverseInertiaLocal",
                "non_finite_motion_inertia",
                "inverseInertiaLocal",
            ),
            (
                "centerOfMassWorld",
                "non_finite_center_of_mass",
                "centerOfMassWorld",
            ),
        ] {
            if let Some(value) = members_get(members, field) {
                if !vec_is_finite(value) {
                    out.push(Violation {
                        rule_id: rule.into(),
                        severity: Severity::Error,
                        body_index: None,
                        shape_index: None,
                        observed: "NaN/inf".into(),
                        expected: "finite".into(),
                        message: format!("motionCinfo {i} {label} contains NaN/inf"),
                    });
                }
            }
        }
    }
}

fn check_structural(
    _psd: &HkxObject,
    _bodies: &[HkxValue],
    motions: &[HkxValue],
    motion_properties: &[HkxValue],
    objects: &[HkxObject],
    out: &mut Vec<Violation>,
) {
    let compressed_shapes = objects
        .iter()
        .filter(|o| o.class_name == "hknpCompressedMeshShape")
        .count();
    let compressed_data = objects
        .iter()
        .filter(|o| o.class_name == "hknpCompressedMeshShapeData")
        .count();
    if compressed_shapes != compressed_data {
        out.push(Violation {
            rule_id: "compressed_shape_data_count_mismatch".into(),
            severity: Severity::Error,
            body_index: None,
            shape_index: None,
            observed: format!("shapes={compressed_shapes} data={compressed_data}"),
            expected: "equal counts".into(),
            message: "hknpCompressedMeshShape count != hknpCompressedMeshShapeData count".into(),
        });
    }

    let bodies_len = _bodies.len();
    if motions.len() > bodies_len {
        out.push(Violation {
            rule_id: "motion_cinfo_count_exceeds_bodies".into(),
            severity: Severity::Warning,
            body_index: None,
            shape_index: None,
            observed: format!("motions={} bodies={bodies_len}", motions.len()),
            expected: "motions <= bodies".into(),
            message: "more motionCinfos than bodyCinfos".into(),
        });
    }

    for (motion_index, motion) in motions.iter().enumerate() {
        let Some(members) = object_members(motion) else {
            continue;
        };
        let Some(motion_properties_id) =
            members_get(members, "motionPropertiesId").and_then(as_i64)
        else {
            continue;
        };
        if motion_properties_id == u16::MAX as i64 {
            continue;
        }
        if motion_properties_id < 0 || motion_properties_id as usize >= motion_properties.len() {
            out.push(Violation {
                rule_id: "motion_properties_id_out_of_range".into(),
                severity: Severity::Error,
                body_index: Some(motion_index),
                shape_index: None,
                observed: format!(
                    "motionPropertiesId={motion_properties_id} properties={}",
                    motion_properties.len()
                ),
                expected: "valid motionProperties index or 65535 sentinel".into(),
                message: format!(
                    "motionCinfo {motion_index} references a missing motion properties entry"
                ),
            });
        }
    }
}

fn vec_is_finite(value: &HkxValue) -> bool {
    match value {
        HkxValue::F32List(xs) => xs.iter().all(|x| x.is_finite()),
        HkxValue::Array(items) => items
            .iter()
            .all(|item| as_f32(item).map(|x| x.is_finite()).unwrap_or(true)),
        _ => true,
    }
}

fn check_body_semantic(
    members: &[HkxMember],
    idx: usize,
    motions: &[HkxValue],
    inv: &Invariants,
    out: &mut Vec<Violation>,
) {
    let motion_count = motions.len();
    let motion_id = members_get(members, "motionId").and_then(as_i64);
    let flags = members_get(members, "flags").and_then(as_i64).unwrap_or(0);
    let is_dynamic = (flags & inv.dynamic_flag_bit) != 0;

    if is_dynamic {
        let motion_index = motion_id
            .and_then(|id| usize::try_from(id).ok())
            .filter(|id| *id < motion_count);
        if motion_index.is_none() {
            out.push(Violation {
                rule_id: "dynamic_body_invalid_motion_id".into(),
                severity: Severity::Error,
                body_index: Some(idx),
                shape_index: None,
                observed: format!("motionId={:?} motions={motion_count}", motion_id),
                expected: format!("0..{motion_count}"),
                message: format!("dynamic body {idx} has out-of-range/invalid motionId"),
            });
        }
        // Mass lives on the motion cinfo as `inverseMass` (body cinfos carry no mass —
        // verified against vanilla FO4). inverseMass == 0 ⇒ infinite mass, the
        // documented physics-freeze case in `multi_body.rs`; < 0 / NaN ⇒ corrupt.
        if let Some(inv_mass) = motion_index
            .and_then(|i| motions.get(i))
            .and_then(object_members)
            .and_then(|m| members_get(m, "inverseMass"))
            .and_then(as_f32)
        {
            if !inv_mass.is_finite() || inv_mass <= 0.0 {
                out.push(Violation {
                    rule_id: "dynamic_body_nonpositive_mass".into(),
                    severity: Severity::Error,
                    body_index: Some(idx),
                    shape_index: None,
                    observed: format!("inverseMass={inv_mass}"),
                    expected: "inverseMass > 0 (finite, positive mass)".into(),
                    message: format!("dynamic body {idx} has non-positive/infinite mass"),
                });
            }
        }
    } else if let Some(id) = motion_id {
        let references_motion = id != inv.invalid_motion_id
            && usize::try_from(id)
                .map(|i| i < motion_count)
                .unwrap_or(false);
        if references_motion {
            out.push(Violation {
                rule_id: "static_body_live_motion_link".into(),
                severity: Severity::Warning,
                body_index: Some(idx),
                shape_index: None,
                observed: format!("motionId={id}"),
                expected: format!("{} (HK_INVALID)", inv.invalid_motion_id),
                message: format!("static body {idx} links a live motionCinfo"),
            });
        }
    }

    if let Some(cfi) = members_get(members, "collisionFilterInfo").and_then(as_i64) {
        let layer = (cfi as u32) & 0xFF;
        if !inv.layers.is_empty() && !inv.layers.contains(&layer) {
            out.push(Violation {
                rule_id: "layer_outside_vanilla_domain".into(),
                severity: Severity::Warning,
                body_index: Some(idx),
                shape_index: None,
                observed: format!("{layer}"),
                expected: format!("{:?}", inv.layers),
                message: format!("body {idx} collision layer not seen in vanilla"),
            });
        }
    }

    if !inv.flags.is_empty() && !inv.flags.contains(&flags) {
        out.push(Violation {
            rule_id: "flags_outside_vanilla_domain".into(),
            severity: Severity::Warning,
            body_index: Some(idx),
            shape_index: None,
            observed: format!("{flags}"),
            expected: format!("{:?}", inv.flags),
            message: format!("body {idx} flags value not seen in vanilla"),
        });
    }

    if let Some(quality) = members_get(members, "qualityId").and_then(as_i64) {
        if !inv.quality_ids.is_empty() && !inv.quality_ids.contains(&quality) {
            out.push(Violation {
                rule_id: "quality_outside_vanilla_domain".into(),
                severity: Severity::Warning,
                body_index: Some(idx),
                shape_index: None,
                observed: format!("{quality}"),
                expected: format!("{:?}", inv.quality_ids),
                message: format!("body {idx} qualityId not seen in vanilla"),
            });
        }
    }
}

fn check_body_finiteness(members: &[HkxMember], idx: usize, out: &mut Vec<Violation>) {
    // Body cinfos carry no `mass` member (verified against vanilla FO4); mass
    // finiteness is checked on the motion cinfo in `check_motion_finiteness`.
    for (field, rule) in [
        ("position", "non_finite_position"),
        ("orientation", "non_finite_orientation"),
    ] {
        if let Some(value) = members_get(members, field) {
            if !vec_is_finite(value) {
                out.push(Violation {
                    rule_id: rule.into(),
                    severity: Severity::Error,
                    body_index: Some(idx),
                    shape_index: None,
                    observed: "NaN/inf".into(),
                    expected: "finite".into(),
                    message: format!("body {idx} {field} contains NaN/inf"),
                });
            }
        }
    }
}

fn shape_index_for_body(objects: &[HkxObject], members: &[HkxMember]) -> Option<usize> {
    members_get(members, "shape").and_then(|v| match v {
        HkxValue::Pointer(idx) => *idx,
        HkxValue::String { value, .. } if !value.is_empty() => objects
            .iter()
            .position(|o| o.name.as_deref() == Some(value.as_str())),
        _ => None,
    })
}

fn vertex_xyz(value: &HkxValue) -> Option<[f32; 3]> {
    match value {
        HkxValue::F32List(xs) if xs.len() >= 3 => Some([xs[0], xs[1], xs[2]]),
        HkxValue::Array(items) if items.len() >= 3 => {
            Some([as_f32(&items[0])?, as_f32(&items[1])?, as_f32(&items[2])?])
        }
        _ => object_members(value).and_then(|m| {
            Some([
                members_get(m, "x").and_then(as_f32)?,
                members_get(m, "y").and_then(as_f32)?,
                members_get(m, "z").and_then(as_f32)?,
            ])
        }),
    }
}

fn check_body_geometry(
    objects: &[HkxObject],
    members: &[HkxMember],
    idx: usize,
    inv: &Invariants,
    out: &mut Vec<Violation>,
) {
    let shape_index = match shape_index_for_body(objects, members) {
        Some(i) => i,
        None => return,
    };
    let shape = match objects.get(shape_index) {
        Some(s) => s,
        None => return,
    };
    if shape.class_name != "hknpConvexPolytopeShape" {
        return;
    }
    let verts: Vec<[f32; 3]> = member(shape, "vertices")
        .and_then(array)
        .map(|items| items.iter().filter_map(vertex_xyz).collect())
        .unwrap_or_default();

    if verts.iter().any(|p| p.iter().any(|x| !x.is_finite())) {
        out.push(Violation {
            rule_id: "non_finite_vertex".into(),
            severity: Severity::Error,
            body_index: Some(idx),
            shape_index: Some(shape_index),
            observed: "NaN/inf".into(),
            expected: "finite".into(),
            message: format!("convex hull for body {idx} has a non-finite vertex"),
        });
        return;
    }

    if verts.len() < inv.min_hull_vertices {
        out.push(Violation {
            rule_id: "degenerate_hull_too_few_vertices".into(),
            severity: Severity::Error,
            body_index: Some(idx),
            shape_index: Some(shape_index),
            observed: format!("{}", verts.len()),
            expected: format!(">= {}", inv.min_hull_vertices),
            message: format!("convex hull for body {idx} has too few vertices"),
        });
        return;
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in &verts {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let extents = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    // Vanilla FO4 ships legitimately thin hulls: flat panels (1 collapsed axis, e.g.
    // barricade walls) and even needles (2 collapsed axes, e.g. BOSLPLeftArmPart04 at
    // [0.77, 4e-5, 3.5e-5]). What vanilla NEVER ships is a hull collapsed to a point
    // (all 3 axes near-zero) — that is genuinely non-buildable. So ERROR only on the
    // point case; a single thin/needle axis with a real largest extent is a WARNING.
    let collapsed_axes = extents
        .iter()
        .filter(|e| **e < inv.degenerate_extent_eps)
        .count();
    let largest = extents.iter().cloned().fold(0.0_f32, f32::max);
    if collapsed_axes == 3 || largest < inv.degenerate_extent_eps {
        out.push(Violation {
            rule_id: "degenerate_hull_collapsed".into(),
            severity: Severity::Error,
            body_index: Some(idx),
            shape_index: Some(shape_index),
            observed: format!("extents={extents:?}"),
            expected: format!("largest axis >= {}", inv.degenerate_extent_eps),
            message: format!("convex hull for body {idx} collapsed to a point / zero-volume"),
        });
    } else {
        let smallest = extents.iter().cloned().fold(f32::INFINITY, f32::min);
        let ratio = if largest > 0.0 {
            smallest / largest
        } else {
            0.0
        };
        if ratio < inv.thin_hull_ratio {
            out.push(Violation {
                rule_id: "degenerate_hull_thin".into(),
                severity: Severity::Warning,
                body_index: Some(idx),
                shape_index: Some(shape_index),
                observed: format!("extents={extents:?} ratio={ratio:.6}"),
                expected: format!("min/max extent ratio >= {}", inv.thin_hull_ratio),
                message: format!("convex hull for body {idx} is very thin (near-planar)"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hkx::model::{HkxMember, HkxObject};
    use crate::hkx::types::HkxValue;

    pub(super) fn obj(class_name: &str, members: Vec<HkxMember>) -> HkxObject {
        HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: class_name.into(),
            members,
        }
    }
    pub(super) fn mem(name: &str, value: HkxValue) -> HkxMember {
        HkxMember {
            name: name.into(),
            value,
        }
    }
    pub(super) fn psd(bodies: Vec<HkxValue>, motions: Vec<HkxValue>) -> Vec<HkxObject> {
        vec![obj(
            "hknpPhysicsSystemData",
            vec![
                mem("bodyCinfos", HkxValue::Array(bodies)),
                mem("motionCinfos", HkxValue::Array(motions)),
            ],
        )]
    }

    fn default_invariants() -> Invariants {
        serde_json::from_str("{}").unwrap()
    }

    #[test]
    fn no_physics_system_data_reports_error() {
        let objects = vec![obj("hknpConvexPolytopeShape", vec![])];
        let v = validate_objects(&objects, &default_invariants());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule_id, "missing_physics_system_data");
        assert_eq!(v[0].severity, Severity::Error);
    }

    #[test]
    fn empty_bodies_no_violations() {
        let objects = psd(vec![], vec![]);
        assert!(validate_objects(&objects, &default_invariants()).is_empty());
    }

    #[test]
    fn invariants_defaults_from_empty_json() {
        let inv = default_invariants();
        assert_eq!(inv.dynamic_flag_bit, 128);
        assert_eq!(inv.invalid_motion_id, 0x7FFF_FFFF);
        assert_eq!(inv.min_hull_vertices, 4);
    }

    fn f32list(values: &[f32]) -> HkxValue {
        HkxValue::F32List(values.to_vec())
    }

    #[test]
    fn compressed_shape_data_count_mismatch_is_error() {
        // One compressed shape but zero shape-data objects.
        let mut objects = psd(vec![], vec![]);
        objects.push(obj("hknpCompressedMeshShape", vec![]));
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            v.iter()
                .any(|x| x.rule_id == "compressed_shape_data_count_mismatch"
                    && x.severity == Severity::Error)
        );
    }

    #[test]
    fn non_finite_position_is_error() {
        let body = HkxValue::Object(vec![mem("position", f32list(&[f32::NAN, 0.0, 0.0, 0.0]))]);
        let objects = psd(vec![body], vec![]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(v.iter().any(|x| x.rule_id == "non_finite_position"
            && x.severity == Severity::Error
            && x.body_index == Some(0)));
    }

    #[test]
    fn finite_position_no_finiteness_violation() {
        let body = HkxValue::Object(vec![
            mem("position", f32list(&[1.0, 2.0, 3.0, 0.0])),
            mem("orientation", f32list(&[0.0, 0.0, 0.0, 1.0])),
        ]);
        let objects = psd(vec![body], vec![]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(!v.iter().any(|x| x.rule_id.starts_with("non_finite")));
    }

    fn body(motion_id: i64, flags: i64, cfi: u32) -> HkxValue {
        HkxValue::Object(vec![
            mem("motionId", HkxValue::I32(motion_id as i32)),
            mem("flags", HkxValue::I32(flags as i32)),
            mem("collisionFilterInfo", HkxValue::U32(cfi)),
        ])
    }

    #[test]
    fn dynamic_body_with_bad_motion_id_is_error() {
        // flags has dynamic bit (128) but motionId is out of range (no motions).
        let objects = psd(vec![body(5, 128, 1)], vec![]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(v.iter().any(
            |x| x.rule_id == "dynamic_body_invalid_motion_id" && x.severity == Severity::Error
        ));
    }

    #[test]
    fn dynamic_body_with_infinite_mass_is_error() {
        // dynamic bit set, motionId valid (one motion), but the motion's inverseMass
        // is 0 ⇒ infinite mass (the documented physics-freeze case).
        let motion = HkxValue::Object(vec![mem("inverseMass", HkxValue::F32(0.0))]);
        let objects = psd(vec![body(0, 128, 1)], vec![motion]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            v.iter()
                .any(|x| x.rule_id == "dynamic_body_nonpositive_mass"
                    && x.severity == Severity::Error)
        );
    }

    #[test]
    fn dynamic_body_with_positive_inverse_mass_is_clean() {
        // dynamic bit set, motionId valid, motion has a real positive inverseMass.
        let motion = HkxValue::Object(vec![mem("inverseMass", HkxValue::F32(0.5))]);
        let objects = psd(vec![body(0, 128, 1)], vec![motion]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            !v.iter()
                .any(|x| x.rule_id == "dynamic_body_nonpositive_mass")
        );
    }

    #[test]
    fn non_finite_motion_inverse_mass_is_error() {
        let motion = HkxValue::TypedObject {
            class_name: "hknpMotionCinfo".into(),
            members: vec![mem("inverseMass", HkxValue::F32(f32::NAN))],
        };
        let objects = psd(vec![], vec![motion]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            v.iter()
                .any(|x| x.rule_id == "non_finite_motion_mass" && x.severity == Severity::Error)
        );
    }

    #[test]
    fn non_finite_motion_inertia_and_com_are_errors() {
        let motion = HkxValue::TypedObject {
            class_name: "hknpMotionCinfo".into(),
            members: vec![
                mem("inverseInertiaLocal", f32list(&[f32::NAN, 0.0, 0.0, 0.0])),
                mem(
                    "centerOfMassWorld",
                    f32list(&[0.0, f32::INFINITY, 0.0, 0.0]),
                ),
            ],
        };
        let objects = psd(vec![], vec![motion]);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            v.iter()
                .any(|x| x.rule_id == "non_finite_motion_inertia" && x.severity == Severity::Error)
        );
        assert!(
            v.iter()
                .any(|x| x.rule_id == "non_finite_center_of_mass" && x.severity == Severity::Error)
        );
    }

    #[test]
    fn static_body_with_invalid_motion_id_is_clean() {
        // static (flags=0), motionId = HK_INVALID, layer in domain.
        let objects = psd(vec![body(0x7FFF_FFFF, 0, 1)], vec![]);
        let inv: Invariants = serde_json::from_str(r#"{"layers":[1],"flags":[0]}"#).unwrap();
        let v = validate_objects(&objects, &inv);
        assert!(!v.iter().any(|x| x.severity == Severity::Error));
    }

    #[test]
    fn layer_outside_domain_is_warning() {
        let objects = psd(vec![body(0x7FFF_FFFF, 0, 200)], vec![]);
        let inv: Invariants = serde_json::from_str(r#"{"layers":[1,2,3],"flags":[0]}"#).unwrap();
        let v = validate_objects(&objects, &inv);
        assert!(v.iter().any(
            |x| x.rule_id == "layer_outside_vanilla_domain" && x.severity == Severity::Warning
        ));
    }

    fn body_with_shape(shape_idx: usize) -> HkxValue {
        HkxValue::Object(vec![
            mem("motionId", HkxValue::I32(0x7FFF_FFFF)),
            mem("flags", HkxValue::I32(0)),
            mem("shape", HkxValue::Pointer(Some(shape_idx))),
        ])
    }

    fn polytope(vertices: &[[f32; 3]]) -> HkxObject {
        let verts: Vec<HkxValue> = vertices
            .iter()
            .map(|p| HkxValue::F32List(vec![p[0], p[1], p[2], 0.0]))
            .collect();
        obj(
            "hknpConvexPolytopeShape",
            vec![mem("vertices", HkxValue::Array(verts))],
        )
    }

    #[test]
    fn single_thin_axis_hull_is_warning_not_error() {
        // A flat panel: extents [1, 1, 0] — one collapsed axis, real area. Vanilla
        // ships these (barricade walls), so it must NOT be an error.
        let shape = polytope(&[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ]);
        let mut objects = psd(vec![body_with_shape(1)], vec![]);
        objects.push(shape);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(!v.iter().any(|x| x.severity == Severity::Error));
        assert!(
            v.iter()
                .any(|x| x.rule_id == "degenerate_hull_thin" && x.severity == Severity::Warning)
        );
    }

    #[test]
    fn needle_hull_two_collapsed_axes_is_not_error() {
        // A vanilla needle: extents [0.77, 4e-5, 3.5e-5] — two collapsed axes but a
        // real largest extent (mirrors BOSLPLeftArmPart04). Must NOT be an error.
        let shape = polytope(&[
            [0.387, -1e-5, 1e-5],
            [0.387, 1e-5, -1e-5],
            [-0.387, 1e-5, 1e-5],
            [-0.387, -1e-5, -1e-5],
        ]);
        let mut objects = psd(vec![body_with_shape(1)], vec![]);
        objects.push(shape);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(!v.iter().any(|x| x.severity == Severity::Error));
    }

    #[test]
    fn collapsed_to_point_hull_is_error() {
        // All three axes near-zero → a point → genuinely non-buildable. Vanilla never
        // ships this, so it is an ERROR.
        let shape = polytope(&[
            [0.0, 0.0, 0.0],
            [1e-6, 0.0, 0.0],
            [0.0, 1e-6, 0.0],
            [0.0, 0.0, 1e-6],
        ]);
        let mut objects = psd(vec![body_with_shape(1)], vec![]);
        objects.push(shape);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            v.iter()
                .any(|x| x.rule_id == "degenerate_hull_collapsed" && x.severity == Severity::Error)
        );
    }

    #[test]
    fn too_few_vertices_is_error() {
        let shape = polytope(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let mut objects = psd(vec![body_with_shape(1)], vec![]);
        objects.push(shape);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(
            v.iter()
                .any(|x| x.rule_id == "degenerate_hull_too_few_vertices"
                    && x.severity == Severity::Error)
        );
    }

    #[test]
    fn solid_hull_is_clean() {
        let shape = polytope(&[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ]);
        let mut objects = psd(vec![body_with_shape(1)], vec![]);
        objects.push(shape);
        let v = validate_objects(&objects, &serde_json::from_str("{}").unwrap());
        assert!(!v.iter().any(|x| x.rule_id.starts_with("degenerate_hull")));
    }

    #[test]
    fn ragdoll_data_root_is_accepted() {
        // A blob rooted at hknpRagdollData (bhkRagdollSystem) must not emit a spurious
        // missing_physics_system_data error.
        let objects = vec![obj(
            "hknpRagdollData",
            vec![
                mem("bodyCinfos", HkxValue::Array(vec![])),
                mem("motionCinfos", HkxValue::Array(vec![])),
            ],
        )];
        let v = validate_objects(&objects, &default_invariants());
        assert!(!v.iter().any(|x| x.rule_id == "missing_physics_system_data"));
    }

    #[test]
    fn motion_properties_id_out_of_range_is_error() {
        let motion = HkxValue::TypedObject {
            class_name: "hknpMotionCinfo".into(),
            members: vec![mem("motionPropertiesId", HkxValue::U16(0))],
        };
        let objects = vec![obj(
            "hknpRagdollData",
            vec![
                mem(
                    "bodyCinfos",
                    HkxValue::Array(vec![HkxValue::Object(vec![])]),
                ),
                mem("motionProperties", HkxValue::Array(vec![])),
                mem("motionCinfos", HkxValue::Array(vec![motion])),
            ],
        )];

        let violations = validate_objects(&objects, &default_invariants());
        assert!(violations.iter().any(|violation| {
            violation.rule_id == "motion_properties_id_out_of_range"
                && violation.severity == Severity::Error
        }));
    }

    #[test]
    fn invalid_motion_properties_sentinel_is_accepted() {
        let motion = HkxValue::TypedObject {
            class_name: "hknpMotionCinfo".into(),
            members: vec![mem("motionPropertiesId", HkxValue::U16(u16::MAX))],
        };
        let objects = vec![obj(
            "hknpRagdollData",
            vec![
                mem(
                    "bodyCinfos",
                    HkxValue::Array(vec![HkxValue::Object(vec![])]),
                ),
                mem("motionProperties", HkxValue::Array(vec![])),
                mem("motionCinfos", HkxValue::Array(vec![motion])),
            ],
        )];

        let violations = validate_objects(&objects, &default_invariants());
        assert!(
            !violations
                .iter()
                .any(|violation| violation.rule_id == "motion_properties_id_out_of_range")
        );
    }
}
