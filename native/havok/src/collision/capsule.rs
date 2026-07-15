// FO4 Havok 2014.1.0 packfile builder — hknpCapsuleShape.
//
// A capsule is NOT a sphere-like primitive: per the Havok 2018 SDK
// (`hknpCapsuleShape : public hknpConvexPolytopeShape`) it is a full convex
// polytope — 8 vertices forming a thin box along the capsule axis — plus the
// two endpoint vectors `m_a` / `m_b`, with the rounding radius carried in
// `convexRadius` (NOT in `m_a.w`, which vanilla leaves at 1.0). The hull
// geometry is generated exactly as `convert::fo76::capsule_hull_from_endpoints`
// does (the converter already emits valid FO4 capsules this way), so this
// builder reproduces the source FO76 capsule faithfully by carrying its own
// `a` / `b` / `convexRadius` — it never synthesizes the radius split.
//
// Build strategy: take a single-body sphere packfile as the system template
// (hknpPhysicsSystemData + one static body + one material, and — like a capsule
// — NO hknpShapeMassProperties), then morph its `hknpSphereShape` into an
// `hknpCapsuleShape` and serialize through the descriptor writer
// (`HkxFile::from_tagxml` → `save`). Unlike the sphere shape (whose
// descriptor-unknown trailer bytes can't round-trip), the capsule is fully
// descriptor-covered, so the writer lays it out correctly.
//
// NOT YET WIRED: `classify_source_body` in the FO76->FO4 conversion still
// tessellates source capsules to hulls rather than routing them here.

use super::compressed_mesh::BuildOptions;
use super::sphere::build_fo4_sphere_collision;
use crate::error::{HavokError, HavokResult};
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

/// `hknpCapsuleShape::CAPSULE_FLAGS` as observed in vanilla FO4 (44ammo.nif):
/// POLYTOPE_FLAGS | USE_SMALL_FACE_INDICES | USE_NORMAL_TO_FIND_SUPPORT_PLANE |
/// SUPPORTS_COLLISIONS_WITH_INTERIOR_TRIANGLES = 0x1C3.
const CAPSULE_FLAGS: u64 = 451;

/// Build a FO4 Havok 2014.1.0 packfile for a single capsule body.
///
/// `a` / `b` are the capsule axis endpoints in Havok space (NIF / havok_scale),
/// carried verbatim from the source FO76 `hknpCapsuleShape` (their `.w` stays
/// 1.0 in vanilla). `convex_radius` is the source capsule's `convexRadius` — the
/// hull half-width is `|a.w| - convex_radius`, so the source's own radius split
/// is preserved rather than re-derived.
///
/// `position` is the body world position (the per-body merge patches it for
/// multi-body systems, so sole callers pass the body origin).
pub fn build_fo4_capsule_collision(
    a: [f32; 4],
    b: [f32; 4],
    convex_radius: f32,
    position: [f32; 3],
    opts: &BuildOptions,
) -> HavokResult<Vec<u8>> {
    let (vertices, planes, faces, indices) =
        capsule_hull(a, b, convex_radius).ok_or_else(|| {
            HavokError::InvalidInput(
                "degenerate capsule: |a.w| <= convexRadius or zero-length axis".to_string(),
            )
        })?;

    // Sphere template radius is irrelevant — the shape is replaced. Keep it
    // positive so the template build can't itself reject a degenerate radius.
    let template_radius = a[3].abs().max(convex_radius).max(0.01);
    let template = build_fo4_sphere_collision(template_radius, position, opts)?;
    let file = HkxFile::read(&template)?;
    let mut objects: Vec<HkxObject> = file.objects().to_vec();

    let shape_idx = objects
        .iter()
        .position(|o| o.class_name == "hknpSphereShape")
        .ok_or_else(|| {
            HavokError::InvalidInput("sphere template missing hknpSphereShape".to_string())
        })?;

    let shape = &mut objects[shape_idx];
    shape.class_name = "hknpCapsuleShape".to_string();
    // Keep the inherited base members (numShapeKeyBits=0, dispatchType=1,
    // userData=material crc) the reader decoded from the sphere's packed flags
    // word; override only what differs for a capsule.
    set_int_preserving(&mut shape.members, "flags", CAPSULE_FLAGS);
    set_value(
        &mut shape.members,
        "convexRadius",
        HkxValue::F32(convex_radius),
    );
    set_value(&mut shape.members, "properties", HkxValue::Pointer(None));
    set_value(&mut shape.members, "vertices", HkxValue::Array(vertices));
    set_value(&mut shape.members, "planes", HkxValue::Array(planes));
    set_value(&mut shape.members, "faces", HkxValue::Array(faces));
    set_value(&mut shape.members, "indices", HkxValue::Array(indices));
    set_value(&mut shape.members, "a", HkxValue::F32List(a.to_vec()));
    set_value(&mut shape.members, "b", HkxValue::F32List(b.to_vec()));

    let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
    Ok(hkx.save())
}

fn set_value(members: &mut Vec<HkxMember>, name: &str, value: HkxValue) {
    if let Some(member) = members.iter_mut().find(|m| m.name == name) {
        member.value = value;
    } else {
        members.push(HkxMember {
            name: name.to_string(),
            value,
        });
    }
}

/// Overwrite an integer member's value while preserving its descriptor-decoded
/// width (so the writer re-packs `flags` at the same byte size the reader saw).
fn set_int_preserving(members: &mut [HkxMember], name: &str, value: u64) {
    if let Some(member) = members.iter_mut().find(|m| m.name == name) {
        member.value = match &member.value {
            HkxValue::U8(_) => HkxValue::U8(value as u8),
            HkxValue::I8(_) => HkxValue::I8(value as i8),
            HkxValue::U16(_) => HkxValue::U16(value as u16),
            HkxValue::I16(_) => HkxValue::I16(value as i16),
            HkxValue::I32(_) => HkxValue::I32(value as i32),
            HkxValue::U64(_) => HkxValue::U64(value),
            _ => HkxValue::U32(value as u32),
        };
    }
}

/// Generate the 8-vertex convex hull (+ planes / faces / indices) for a capsule
/// axis `a`-`b` with rounding `convex_radius`. Ported verbatim from
/// `convert::fo76::capsule_hull_from_endpoints` so the conversion's existing
/// capsule geometry and this builder stay identical. Returns `None` for a
/// degenerate capsule (axis half-extent <= 1e-7 or zero-length axis).
fn capsule_hull(
    a: [f32; 4],
    b: [f32; 4],
    convex_radius: f32,
) -> Option<(Vec<HkxValue>, Vec<HkxValue>, Vec<HkxValue>, Vec<HkxValue>)> {
    const FLT_MIN: f32 = -3.402_823_5e38;
    const FACE_INDICES: [u8; 24] = [
        2, 6, 4, 0, 1, 5, 7, 3, 1, 0, 4, 5, 7, 6, 2, 3, 3, 2, 0, 1, 7, 5, 4, 6,
    ];

    let half = (a[3].abs() - convex_radius).abs();
    if half <= 1e-7 {
        return None;
    }

    let start = [b[0], b[1], b[2]];
    let end = [a[0], a[1], a[2]];
    let axis = normalize3(sub3(end, start))?;
    let xy_len = (axis[0] * axis[0] + axis[1] * axis[1]).sqrt();
    let side = if xy_len > 1e-7 {
        [axis[1] / xy_len, -axis[0] / xy_len, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let up = normalize3(cross3(side, axis))?;
    let p0 = sub3(start, scale3(axis, half));
    let p1 = add3(end, scale3(axis, half));

    let vertices = [
        add3(add3(p0, scale3(side, half)), scale3(up, half)),
        add3(add3(p1, scale3(side, half)), scale3(up, half)),
        add3(sub3(p0, scale3(side, half)), scale3(up, half)),
        add3(sub3(p1, scale3(side, half)), scale3(up, half)),
        sub3(add3(p0, scale3(side, half)), scale3(up, half)),
        sub3(add3(p1, scale3(side, half)), scale3(up, half)),
        sub3(sub3(p0, scale3(side, half)), scale3(up, half)),
        sub3(sub3(p1, scale3(side, half)), scale3(up, half)),
    ]
    .into_iter()
    .map(|v| HkxValue::F32List(vec![v[0], v[1], v[2], 0.5]))
    .collect();

    let plane_specs = [
        (scale3(axis, -1.0), p0),
        (axis, p1),
        (side, add3(p0, scale3(side, half))),
        (scale3(side, -1.0), sub3(p0, scale3(side, half))),
        (up, add3(p0, scale3(up, half))),
        (scale3(up, -1.0), sub3(p0, scale3(up, half))),
    ];
    let mut planes: Vec<HkxValue> = plane_specs
        .into_iter()
        .map(|(normal, point)| {
            HkxValue::F32List(vec![normal[0], normal[1], normal[2], -dot3(normal, point)])
        })
        .collect();
    planes.push(HkxValue::F32List(vec![0.0, 0.0, 0.0, FLT_MIN]));
    planes.push(HkxValue::F32List(vec![0.0, 0.0, 0.0, FLT_MIN]));

    let faces = (0..6)
        .map(|i| {
            HkxValue::Object(vec![
                HkxMember {
                    name: "firstIndex".to_string(),
                    value: HkxValue::U16((i * 4) as u16),
                },
                HkxMember {
                    name: "numIndices".to_string(),
                    value: HkxValue::U8(4),
                },
                HkxMember {
                    name: "minHalfAngle".to_string(),
                    value: HkxValue::U8(4),
                },
            ])
        })
        .collect();
    let indices = FACE_INDICES.into_iter().map(HkxValue::U8).collect();

    Some((vertices, planes, faces, indices))
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale3(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot3(v, v).sqrt();
    if len <= 1e-7 {
        None
    } else {
        Some(scale3(v, 1.0 / len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z_capsule() -> Vec<u8> {
        // Capsule along +z, radius split between a thin box and convexRadius.
        let a = [0.0, 0.0, 0.5, 1.0];
        let b = [0.0, 0.0, 0.0, 1.0];
        build_fo4_capsule_collision(a, b, 0.05, [0.0, 0.0, 0.0], &BuildOptions::default())
            .expect("build capsule")
    }

    #[test]
    fn capsule_blob_parses_as_capsule_shape() {
        let blob = z_capsule();
        let file = HkxFile::read(&blob).expect("parse capsule packfile");
        let classes: Vec<&str> = file
            .objects()
            .iter()
            .map(|o| o.class_name.as_str())
            .collect();
        assert!(
            classes.contains(&"hknpCapsuleShape"),
            "expected hknpCapsuleShape, got {classes:?}"
        );
        assert!(
            !classes.contains(&"hknpSphereShape"),
            "sphere template shape must be fully morphed away"
        );
        assert!(classes.contains(&"hknpPhysicsSystemData"));
    }

    #[test]
    fn capsule_carries_endpoints_radius_and_hull() {
        let blob = z_capsule();
        let file = HkxFile::read(&blob).expect("parse");
        let shape = file
            .objects()
            .iter()
            .find(|o| o.class_name == "hknpCapsuleShape")
            .expect("capsule shape present");

        let member = |name: &str| {
            shape
                .members
                .iter()
                .find(|m| m.name == name)
                .map(|m| &m.value)
        };

        // Endpoints carried verbatim (a.w stays 1.0, radius is NOT in a.w).
        match member("a").expect("a member") {
            HkxValue::F32List(v) => {
                assert!((v[2] - 0.5).abs() < 1e-5, "a.z");
                assert!((v[3] - 1.0).abs() < 1e-5, "a.w stays 1.0");
            }
            other => panic!("a not F32List: {other:?}"),
        }
        // convexRadius carries the source split.
        match member("convexRadius").expect("convexRadius") {
            HkxValue::F32(r) => assert!((r - 0.05).abs() < 1e-6),
            other => panic!("convexRadius not F32: {other:?}"),
        }
        // 8-vertex hull, 8 planes, 6 faces, 24 indices (capsule convex polytope).
        match member("vertices").expect("vertices") {
            HkxValue::Array(v) => assert_eq!(v.len(), 8),
            other => panic!("vertices: {other:?}"),
        }
        match member("planes").expect("planes") {
            HkxValue::Array(v) => assert_eq!(v.len(), 8),
            other => panic!("planes: {other:?}"),
        }
        match member("faces").expect("faces") {
            HkxValue::Array(v) => assert_eq!(v.len(), 6),
            other => panic!("faces: {other:?}"),
        }
        match member("indices").expect("indices") {
            HkxValue::Array(v) => assert_eq!(v.len(), 24),
            other => panic!("indices: {other:?}"),
        }
    }

    #[test]
    fn capsule_blob_round_trips_through_writer() {
        // Build -> read -> re-save -> read again must be stable (the morphed
        // PSD/body/material from the sphere template must serialize cleanly).
        let blob = z_capsule();
        let file = HkxFile::read(&blob).expect("parse 1");
        let objects = file.objects().to_vec();
        let resaved = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects).save();
        let file2 = HkxFile::read(&resaved).expect("parse 2");
        assert!(
            file2
                .objects()
                .iter()
                .any(|o| o.class_name == "hknpCapsuleShape")
        );
    }

    #[test]
    fn degenerate_capsule_is_rejected() {
        // |a.w| - convexRadius == 0 -> no hull.
        let a = [0.0, 0.0, 0.5, 0.05];
        let b = [0.0, 0.0, 0.0, 0.05];
        let err = build_fo4_capsule_collision(a, b, 0.05, [0.0; 3], &BuildOptions::default())
            .unwrap_err();
        assert!(matches!(err, HavokError::InvalidInput(_)));
    }
}
