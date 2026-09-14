use std::collections::{HashMap, HashSet};

use super::compound::{CompoundChild, CompoundChildKind};
use super::mass_properties::CompressedMassProperties;
use super::multi_body::MultiBodyShape;
use super::polytope::SourcePolytopeShape;
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

/// Starfield ships Havok 2019.2.0 hknp; the 2018 SDK is the closest published
/// layout reference and writes the same TAG0 revision.
const SUPPORTED_SDK_PREFIXES: &[&str] = &["2018", "2019"];

/// `hknpLevelOfDetail::MAXIMUM` — the finest variant an `hknpLodShape` carries.
const LOD_VARIANT_MAXIMUM: usize = 0;

/// `hkcdStaticMeshTree` pages the shared-vertex pool in 64k-element blocks.
const SHARED_VERTICES_PAGE_SIZE: usize = 65536;

/// Decode the hknp collision graph embedded in a Starfield NIF's
/// `bhkPhysicsSystem` payload into source shapes, one entry per
/// `hknpPhysicsSystemData` body, for `build_fo4_multi_body_collision`.
///
/// # Coordinate space
///
/// Output is Havok space (metres) exactly as stored; no scale is applied.
/// `havok_scale` is NIF units per Havok unit (1.0 Starfield, 69.99125 FO4), so
/// the values are already FO4-ready, as with the FO76 decoders. Fallback
/// collision built from converted render geometry is in FO4 NIF space and must
/// be divided by 69.99125 once before it joins these shapes.
///
/// An unsupported shape class returns `Err` naming the class, so callers can
/// fall back and ship partial coverage.
pub fn decode_starfield_collision(tag0_payload: &[u8]) -> Result<Vec<MultiBodyShape>, String> {
    if tag0_payload.len() < 8 || &tag0_payload[4..8] != b"TAG0" {
        return Err("input is not an embedded TAG0 tagfile".to_string());
    }
    let tagfile = crate::hkx::parse_tagfile(tag0_payload).map_err(|error| error.to_string())?;
    if !SUPPORTED_SDK_PREFIXES
        .iter()
        .any(|prefix| tagfile.sdk_version.starts_with(prefix))
    {
        return Err(format!(
            "unsupported tagfile SDK version {}",
            tagfile.sdk_version
        ));
    }
    let hkx = tagfile
        .materialize_hkx()
        .map_err(|error| error.to_string())?;

    let system = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .ok_or_else(|| "tagfile has no hknpPhysicsSystemData".to_string())?;
    let bodies = member_array(&system.members, "bodyCinfos")
        .ok_or_else(|| "hknpPhysicsSystemData has no bodyCinfos array".to_string())?;
    if bodies.is_empty() {
        return Err("hknpPhysicsSystemData has no bodies".to_string());
    }

    let mut shapes = Vec::with_capacity(bodies.len());
    for (body_index, body) in bodies.iter().enumerate() {
        let members = body
            .as_object_members()
            .ok_or_else(|| format!("body {body_index} is not an inline object"))?;
        let shape_index = pointer(members, "shape")
            .ok_or_else(|| format!("body {body_index} has no shape pointer"))?;
        let mut visiting = HashSet::new();
        shapes.push(decode_shape(&hkx, shape_index, &mut visiting)?);
    }
    Ok(shapes)
}

fn decode_shape(
    hkx: &HkxFile,
    shape_index: usize,
    visiting: &mut HashSet<usize>,
) -> Result<MultiBodyShape, String> {
    if !visiting.insert(shape_index) {
        return Err(format!("shape {shape_index} is part of a reference cycle"));
    }
    let result = decode_shape_inner(hkx, shape_index, visiting);
    visiting.remove(&shape_index);
    result
}

fn decode_shape_inner(
    hkx: &HkxFile,
    shape_index: usize,
    visiting: &mut HashSet<usize>,
) -> Result<MultiBodyShape, String> {
    let object = hkx
        .objects()
        .get(shape_index)
        .ok_or_else(|| format!("shape pointer {shape_index} is out of range"))?;
    match object.class_name.as_str() {
        "hknpLodShape" => {
            let variant = lod_variant_index(object).ok_or_else(|| {
                format!("hknpLodShape {shape_index} has no resolvable variant shape")
            })?;
            decode_shape(hkx, variant, visiting)
        }
        "hknpSphereShape" => {
            let center = *hull_vertices(object)
                .first()
                .ok_or_else(|| format!("hknpSphereShape {shape_index} has no hull vertex"))?;
            let radius = f32_member(&object.members, "convexRadius").unwrap_or(0.0);
            if !radius.is_finite() || radius <= 0.0 || !center.iter().all(|v| v.is_finite()) {
                return Err(format!(
                    "hknpSphereShape {shape_index} has invalid geometry"
                ));
            }
            Ok(MultiBodyShape::Sphere {
                radius,
                position: center,
            })
        }
        // Starfield's hknpCapsuleShape adds no members of its own over
        // hknpConvexShape: measured across every instance under meshes/actors,
        // the hull holds exactly the two endpoint vertices and no planes, faces
        // or indices, and there are no `a`/`b` members like FO76's 2015-era
        // class had. `SourceCapsuleShape` needs a fully-populated polytope hull,
        // so there is nothing to build one from. Every instance sits on an actor
        // ragdoll skeleton, which the world-only scope excludes anyway.
        "hknpCapsuleShape" => Err(format!(
            "unsupported hknp shape class hknpCapsuleShape at shape {shape_index}"
        )),
        "hknpConvexShape" | "hknpBoxShape" | "hknpCylinderShape" | "hknpTriangleShape" => {
            Ok(MultiBodyShape::SourcePolytope {
                shape: source_polytope(hkx, object, shape_index)?,
            })
        }
        "hknpCompressedMeshShape" => {
            let (vertices, triangles) = compressed_mesh_geometry(hkx, object, shape_index)?;
            Ok(MultiBodyShape::CompressedMesh {
                vertices,
                triangles,
            })
        }
        "hknpCompoundShape" => Ok(MultiBodyShape::Compound {
            children: compound_children(hkx, object, shape_index, visiting)?,
        }),
        other => Err(format!(
            "unsupported hknp shape class {other} at shape {shape_index}"
        )),
    }
}

/// `hknpLodShape::m_variants` is a fixed 8-slot table indexed by
/// `hknpLevelOfDetail::Enum`; slot 0 (`MAXIMUM`) holds the finest shape, which
/// is the one FO4 statics want. Coarser slots repeat the same pointer when a
/// LOD was never authored, so falling forward to the first populated slot is
/// safe.
fn lod_variant_index(object: &HkxObject) -> Option<usize> {
    let variants = member_array(&object.members, "variants")?;
    let pointer_at = |index: usize| match variants.get(index) {
        Some(HkxValue::Pointer(target)) => *target,
        _ => None,
    };
    pointer_at(LOD_VARIANT_MAXIMUM).or_else(|| (0..variants.len()).find_map(pointer_at))
}

fn compound_children(
    hkx: &HkxFile,
    object: &HkxObject,
    shape_index: usize,
    visiting: &mut HashSet<usize>,
) -> Result<Vec<CompoundChild>, String> {
    let instances = object_member(&object.members, "instances")
        .and_then(|members| member_array(members, "elements"))
        .ok_or_else(|| format!("hknpCompoundShape {shape_index} has no instance elements"))?;
    let mut children = Vec::new();
    for (element_index, element) in instances.iter().enumerate() {
        let members = element.as_object_members().ok_or_else(|| {
            format!("hknpCompoundShape {shape_index} instance {element_index} is not an object")
        })?;
        let Some(child_index) = pointer(members, "shape") else {
            continue;
        };
        children.push(CompoundChild {
            transform: instance_transform(members),
            kind: compound_child_kind(hkx, child_index, visiting)?,
        });
    }
    if children.is_empty() {
        return Err(format!(
            "hknpCompoundShape {shape_index} has no convertible children"
        ));
    }
    Ok(children)
}

fn compound_child_kind(
    hkx: &HkxFile,
    child_index: usize,
    visiting: &mut HashSet<usize>,
) -> Result<CompoundChildKind, String> {
    if !visiting.insert(child_index) {
        return Err(format!(
            "compound child {child_index} is part of a reference cycle"
        ));
    }
    let result = (|| {
        let child = hkx
            .objects()
            .get(child_index)
            .ok_or_else(|| format!("compound child pointer {child_index} is out of range"))?;
        match child.class_name.as_str() {
            "hknpLodShape" => {
                let variant = lod_variant_index(child).ok_or_else(|| {
                    format!("hknpLodShape {child_index} has no resolvable variant shape")
                })?;
                compound_child_kind(hkx, variant, visiting)
            }
            "hknpConvexShape" | "hknpBoxShape" | "hknpCylinderShape" | "hknpTriangleShape" => {
                Ok(CompoundChildKind::SourcePolytope {
                    shape: source_polytope(hkx, child, child_index)?,
                })
            }
            "hknpCompressedMeshShape" => {
                let (vertices, triangles) = compressed_mesh_geometry(hkx, child, child_index)?;
                Ok(CompoundChildKind::CompressedMesh {
                    vertices,
                    triangles,
                })
            }
            other => Err(format!(
                "unsupported hknp compound child class {other} at shape {child_index}"
            )),
        }
    })();
    visiting.remove(&child_index);
    result
}

/// Compose the instance's quaternion rotation, per-axis scale and translation
/// into the row-major local-to-compound matrix `CompoundChild` expects.
fn instance_transform(members: &[HkxMember]) -> [[f32; 4]; 4] {
    let rotation = vec4_member(members, "rotation").unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let translation = float3_member(members, "translation").unwrap_or([0.0; 3]);
    let scale = float3_member(members, "scale").unwrap_or([1.0; 3]);
    let basis = quaternion_basis(rotation);
    let mut transform = CompoundChild::identity_transform();
    for row in 0..3 {
        for column in 0..3 {
            transform[row][column] = basis[row][column] * scale[column];
        }
        transform[row][3] = translation[row];
    }
    transform
}

fn quaternion_basis([x, y, z, w]: [f32; 4]) -> [[f32; 3]; 3] {
    let length_squared = x * x + y * y + z * z + w * w;
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let scale = 2.0 / length_squared;
    let (xs, ys, zs) = (x * scale, y * scale, z * scale);
    let (wx, wy, wz) = (w * xs, w * ys, w * zs);
    let (xx, xy, xz) = (x * xs, x * ys, x * zs);
    let (yy, yz, zz) = (y * ys, y * zs, z * zs);
    [
        [1.0 - (yy + zz), xy - wz, xz + wy],
        [xy + wz, 1.0 - (xx + zz), yz - wx],
        [xz - wy, yz + wx, 1.0 - (xx + yy)],
    ]
}

/// Decode an `hknpCompressedMeshShape` to a plain vertex/triangle soup in Havok
/// space, which the shared FO4 writer re-compresses natively.
///
/// The vertex codec matches FO76 (11-11-10 section-local packing plus a 21-21-22
/// object-space shared pool), but the 2018 SDK flattened
/// `hkcdStaticMeshTree::Section`: FO76 packs the shared/primitive/data-run
/// cursors as `index << 8 | count` in nested structs with a per-section `domain`
/// and Aabb4 node array; Starfield has explicit `first*Index`/`num*` fields and
/// no per-section AABB tree. Raw geometry avoids synthesizing section AABBs and
/// tree nodes for `RawCompressedMeshData`, at the cost of per-triangle source materials.
fn compressed_mesh_geometry(
    hkx: &HkxFile,
    object: &HkxObject,
    shape_index: usize,
) -> Result<(Vec<[f32; 3]>, Vec<[u32; 3]>), String> {
    let fail = |reason: &str| format!("hknpCompressedMeshShape {shape_index}: {reason}");
    let data = hkx
        .objects()
        .get(pointer(&object.members, "data").ok_or_else(|| fail("no data pointer"))?)
        .ok_or_else(|| fail("data pointer is out of range"))?;
    let mesh_tree = object_member(&data.members, "meshTree")
        .ok_or_else(|| fail("shape data has no meshTree"))?;
    let domain =
        object_member(mesh_tree, "domain").ok_or_else(|| fail("meshTree has no domain"))?;
    let object_min = float3_member(domain, "min").ok_or_else(|| fail("domain has no min"))?;
    let object_max = float3_member(domain, "max").ok_or_else(|| fail("domain has no max"))?;

    let packed_vertices = u32_array(mesh_tree, "packedVertices");
    let shared_vertices = u64_array(mesh_tree, "sharedVertices");
    let shared_index = u32_array(mesh_tree, "sharedVerticesIndex");
    let primitives = member_array(mesh_tree, "primitives").unwrap_or(&[]);
    let sections = member_array(mesh_tree, "sections").ok_or_else(|| fail("no sections"))?;

    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    for section_value in sections {
        let section = section_value
            .as_object_members()
            .ok_or_else(|| fail("section is not an inline object"))?;
        let codec: Vec<f32> = member_array(section, "codecParms")
            .unwrap_or(&[])
            .iter()
            .filter_map(f32_value)
            .collect();
        if codec.len() < 6 {
            return Err(fail("section codecParms is not six floats"));
        }
        let base = [codec[0], codec[1], codec[2]];
        let scale = [codec[3], codec[4], codec[5]];
        let page = u32_member(section, "page").unwrap_or(0) as usize;
        let first_packed = u32_member(section, "firstPackedVertexIndex").unwrap_or(0) as usize;
        let num_packed = u32_member(section, "numPackedVertices").unwrap_or(0) as usize;
        let first_shared = u32_member(section, "firstSharedVertexIndex").unwrap_or(0) as usize;
        let first_primitive = u32_member(section, "firstPrimitiveIndex").unwrap_or(0) as usize;
        let num_primitives = u32_member(section, "numPrimitives").unwrap_or(0) as usize;

        let section_packed = packed_vertices
            .get(first_packed..first_packed + num_packed)
            .ok_or_else(|| fail("section packed-vertex range is out of bounds"))?;
        let section_primitives = primitives
            .get(first_primitive..first_primitive + num_primitives)
            .ok_or_else(|| fail("section primitive range is out of bounds"))?;

        let mut local_to_vertex: Vec<u32> = Vec::with_capacity(num_packed);
        for &packed in section_packed {
            local_to_vertex.push(vertices.len() as u32);
            vertices.push(decode_packed_vertex(packed, base, scale));
        }
        let mut shared_cache: HashMap<usize, u32> = HashMap::new();

        for primitive_value in section_primitives {
            let members = primitive_value
                .as_object_members()
                .ok_or_else(|| fail("primitive is not an inline object"))?;
            let indices = member_array(members, "indices").unwrap_or(&[]);
            if indices.len() < 4 {
                return Err(fail("primitive has fewer than 4 indices"));
            }
            let quad: Vec<usize> = indices
                .iter()
                .take(4)
                .filter_map(|value| usize::try_from(integer(value)?).ok())
                .collect();
            if quad.len() < 4 {
                return Err(fail("primitive indices are not integers"));
            }
            let (a, b, c, d) = (quad[0], quad[1], quad[2], quad[3]);
            // hkcdStaticMeshTree::Primitive::getType — b == d tags a CUSTOM
            // (flat-convex) primitive whose vertices live in the shared pool
            // behind a record header rather than being indexable directly.
            if b == d {
                continue;
            }
            let mut corners = [0u32; 4];
            let corner_count = if c == d { 3 } else { 4 };
            let mut resolved = true;
            for (slot, local) in corners.iter_mut().zip([a, b, c, d]).take(corner_count) {
                match resolve_section_vertex(
                    local,
                    num_packed,
                    first_shared,
                    page,
                    &local_to_vertex,
                    &shared_index,
                    &shared_vertices,
                    object_min,
                    object_max,
                    &mut shared_cache,
                    &mut vertices,
                ) {
                    Some(index) => *slot = index,
                    None => {
                        resolved = false;
                        break;
                    }
                }
            }
            if !resolved {
                continue;
            }
            triangles.push([corners[0], corners[1], corners[2]]);
            if corner_count == 4 {
                triangles.push([corners[0], corners[2], corners[3]]);
            }
        }
    }

    if vertices.is_empty() || triangles.is_empty() {
        return Err(fail("decoded no triangles"));
    }
    Ok((vertices, triangles))
}

/// Section-local vertex indices below `num_packed` address the section's packed
/// block; above it they address the object-wide shared pool through the
/// section's `sharedVerticesIndex` window.
#[allow(clippy::too_many_arguments)]
fn resolve_section_vertex(
    local: usize,
    num_packed: usize,
    first_shared: usize,
    page: usize,
    local_to_vertex: &[u32],
    shared_index: &[u32],
    shared_vertices: &[u64],
    object_min: [f32; 3],
    object_max: [f32; 3],
    shared_cache: &mut HashMap<usize, u32>,
    vertices: &mut Vec<[f32; 3]>,
) -> Option<u32> {
    if local < num_packed {
        return local_to_vertex.get(local).copied();
    }
    let shared_slot = first_shared + (local - num_packed);
    if let Some(index) = shared_cache.get(&shared_slot) {
        return Some(*index);
    }
    let pool_index = page * SHARED_VERTICES_PAGE_SIZE + *shared_index.get(shared_slot)? as usize;
    let packed = *shared_vertices.get(pool_index)?;
    let index = vertices.len() as u32;
    vertices.push(decode_shared_vertex(packed, object_min, object_max));
    shared_cache.insert(shared_slot, index);
    Some(index)
}

fn decode_packed_vertex(packed: u32, base: [f32; 3], scale: [f32; 3]) -> [f32; 3] {
    let (x, y, z) = super::unpack_vertex_11_11_10(packed);
    [
        base[0] + x as f32 * scale[0],
        base[1] + y as f32 * scale[1],
        base[2] + z as f32 * scale[2],
    ]
}

fn decode_shared_vertex(packed: u64, min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    let (x, y, z) = super::unpack_vertex_21_21_22(packed);
    let step = |lo: f32, hi: f32, mask: u64| {
        if hi > lo {
            (hi - lo) / mask as f32
        } else {
            0.0
        }
    };
    [
        min[0] + x as f32 * step(min[0], max[0], (1 << 21) - 1),
        min[1] + y as f32 * step(min[1], max[1], (1 << 21) - 1),
        min[2] + z as f32 * step(min[2], max[2], (1 << 22) - 1),
    ]
}

/// Starfield nests the hull geometry in an `hknpConvexHull` sub-struct; FO76's
/// `hknpConvexPolytopeShape` carried the same arrays as direct members.
fn source_polytope(
    hkx: &HkxFile,
    object: &HkxObject,
    shape_index: usize,
) -> Result<SourcePolytopeShape, String> {
    let hull = object_member(&object.members, "hull")
        .ok_or_else(|| format!("{} {shape_index} has no hull", object.class_name))?;
    let faces = member_array(hull, "faces")
        .unwrap_or(&[])
        .iter()
        .filter_map(|value| {
            let members = value.as_object_members()?;
            Some((
                u16::try_from(integer(members_value(members, "firstIndex")?)?).ok()?,
                u8::try_from(integer(members_value(members, "numIndices")?)?).ok()?,
                u8::try_from(integer(members_value(members, "minHalfAngle")?)?).ok()?,
            ))
        })
        .collect();
    let shape = SourcePolytopeShape {
        vertices: hull_vertices(object),
        planes: member_array(hull, "planes")
            .unwrap_or(&[])
            .iter()
            .filter_map(vec4)
            .collect(),
        faces,
        indices: member_array(hull, "indices")
            .unwrap_or(&[])
            .iter()
            .filter_map(|value| u8::try_from(integer(value)?).ok())
            .collect(),
        convex_radius: f32_member(&object.members, "convexRadius").unwrap_or(0.0),
        mass_properties: shape_mass_properties(hkx, object),
    };
    shape
        .validate()
        .map_err(|error| format!("{} {shape_index}: {error}", object.class_name))?;
    Ok(shape)
}

fn hull_vertices(object: &HkxObject) -> Vec<[f32; 3]> {
    object_member(&object.members, "hull")
        .and_then(|hull| member_array(hull, "vertices"))
        .unwrap_or(&[])
        .iter()
        .filter_map(float3)
        .collect()
}

fn shape_mass_properties(hkx: &HkxFile, object: &HkxObject) -> Option<CompressedMassProperties> {
    let properties = hkx.objects().get(pointer(&object.members, "properties")?)?;
    for entry in member_array(&properties.members, "entries")? {
        let members = entry.as_object_members()?;
        let Some(target) = pointer(members, "object") else {
            continue;
        };
        let candidate = hkx.objects().get(target)?;
        if candidate.class_name != "hknpShapeMassProperties" {
            continue;
        }
        let compressed = object_member(&candidate.members, "compressedMassProperties")?;
        return Some(CompressedMassProperties {
            center_of_mass: packed_i16x4(compressed, "centerOfMass")?,
            inertia: packed_i16x4(compressed, "inertia")?,
            major_axis_space: i16x4(member_array(compressed, "majorAxisSpace")?)?,
            mass: f32_member(compressed, "mass").unwrap_or(0.0),
            volume: f32_member(compressed, "volume").unwrap_or(0.0),
        });
    }
    None
}

/// `hkPackedVector3` wraps its four half-precision lanes in a `values` array.
fn packed_i16x4(members: &[HkxMember], name: &str) -> Option<[i16; 4]> {
    i16x4(member_array(object_member(members, name)?, "values")?)
}

fn i16x4(values: &[HkxValue]) -> Option<[i16; 4]> {
    let mut out = [0i16; 4];
    if values.len() < 4 {
        return None;
    }
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = i16::try_from(integer(value)?).ok()?;
    }
    Some(out)
}

fn members_value<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a HkxValue> {
    members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn member_array<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a [HkxValue]> {
    match members_value(members, name)? {
        HkxValue::Array(values) => Some(values),
        _ => None,
    }
}

fn object_member<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a [HkxMember]> {
    members_value(members, name)?.as_object_members()
}

fn pointer(members: &[HkxMember], name: &str) -> Option<usize> {
    match members_value(members, name)? {
        HkxValue::Pointer(target) => *target,
        _ => None,
    }
}

fn f32_member(members: &[HkxMember], name: &str) -> Option<f32> {
    f32_value(members_value(members, name)?)
}

fn f32_value(value: &HkxValue) -> Option<f32> {
    match value {
        HkxValue::F32(value) | HkxValue::Half(value) => Some(*value),
        _ => None,
    }
}

fn u32_member(members: &[HkxMember], name: &str) -> Option<u32> {
    u32::try_from(integer(members_value(members, name)?)?).ok()
}

fn u32_array(members: &[HkxMember], name: &str) -> Vec<u32> {
    member_array(members, name)
        .unwrap_or(&[])
        .iter()
        .filter_map(|value| u32::try_from(integer(value)?).ok())
        .collect()
}

fn u64_array(members: &[HkxMember], name: &str) -> Vec<u64> {
    member_array(members, name)
        .unwrap_or(&[])
        .iter()
        .filter_map(|value| match value {
            HkxValue::U64(bits) => Some(*bits),
            HkxValue::I64(bits) => Some(*bits as u64),
            other => u64::try_from(integer(other)?).ok(),
        })
        .collect()
}

fn float3_member(members: &[HkxMember], name: &str) -> Option<[f32; 3]> {
    float3(members_value(members, name)?)
}

fn vec4_member(members: &[HkxMember], name: &str) -> Option<[f32; 4]> {
    vec4(members_value(members, name)?)
}

fn float3(value: &HkxValue) -> Option<[f32; 3]> {
    match value {
        HkxValue::F32List(values) if values.len() >= 3 => Some([values[0], values[1], values[2]]),
        _ => {
            let members = value.as_object_members()?;
            Some([
                f32_member(members, "x")?,
                f32_member(members, "y")?,
                f32_member(members, "z")?,
            ])
        }
    }
}

fn vec4(value: &HkxValue) -> Option<[f32; 4]> {
    match value {
        HkxValue::F32List(values) if values.len() >= 4 => {
            Some([values[0], values[1], values[2], values[3]])
        }
        _ => {
            let members = value.as_object_members()?;
            Some([
                f32_member(members, "x")?,
                f32_member(members, "y")?,
                f32_member(members, "z")?,
                f32_member(members, "w").unwrap_or(0.0),
            ])
        }
    }
}

fn integer(value: &HkxValue) -> Option<i64> {
    match value {
        HkxValue::I8(v) => Some(i64::from(*v)),
        HkxValue::U8(v) => Some(i64::from(*v)),
        HkxValue::I16(v) => Some(i64::from(*v)),
        HkxValue::U16(v) => Some(i64::from(*v)),
        HkxValue::I32(v) => Some(i64::from(*v)),
        HkxValue::U32(v) => Some(i64::from(*v)),
        HkxValue::I64(v) => Some(*v),
        HkxValue::U64(v) => i64::try_from(*v).ok(),
        HkxValue::Bool(v) => Some(i64::from(*v)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_tag0_input() {
        let error = decode_starfield_collision(b"not a tagfile at all").unwrap_err();
        assert!(error.contains("not an embedded TAG0"), "{error}");
    }

    #[test]
    fn identity_quaternion_yields_identity_basis() {
        assert_eq!(
            quaternion_basis([0.0, 0.0, 0.0, 1.0]),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
    }

    #[test]
    fn instance_transform_places_translation_in_last_column() {
        let members = vec![
            HkxMember {
                name: "rotation".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0]),
            },
            HkxMember {
                name: "translation".to_string(),
                value: HkxValue::F32List(vec![1.0, 2.0, 3.0]),
            },
            HkxMember {
                name: "scale".to_string(),
                value: HkxValue::F32List(vec![2.0, 2.0, 2.0]),
            },
        ];
        let transform = instance_transform(&members);
        assert_eq!(
            transform,
            [
                [2.0, 0.0, 0.0, 1.0],
                [0.0, 2.0, 0.0, 2.0],
                [0.0, 0.0, 2.0, 3.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
    }
}
