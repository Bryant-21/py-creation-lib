use std::collections::HashSet;

use serde_json::json;

use super::capsule::SourceCapsuleShape;
use super::compound::{CompoundChild, CompoundChildKind};
use super::compressed_mesh::{
    MaterialEntry, RawCompressedMeshBitField, RawCompressedMeshData, RawCompressedMeshDataRun,
    RawCompressedMeshSection, RawCompressedMeshSparseMap,
};
use super::convex::SourceConvexShape;
use super::mass_properties::SourceMassDistribution;
use super::polytope::SourcePolytopeShape;
use crate::error::{HavokError, HavokResult};
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceBodyTransform {
    pub position: [f32; 4],
    pub orientation: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewMesh {
    pub shape_type: String,
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourcePrimitiveShape {
    Sphere { center: [f32; 3], radius: f32 },
    Capsule(SourceCapsuleShape),
    Convex(SourceConvexShape),
}

#[derive(Debug, Clone, Copy)]
struct ShapeTransform {
    basis: [[f32; 3]; 3],
    translation: [f32; 3],
}

impl ShapeTransform {
    fn identity() -> Self {
        Self {
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [0.0, 0.0, 0.0],
        }
    }

    fn is_identity(self) -> bool {
        self.basis == Self::identity().basis && self.translation == [0.0, 0.0, 0.0]
    }

    fn compose(self, child: Self) -> Self {
        let mut basis = [[0.0; 3]; 3];
        for row in 0..3 {
            for col in 0..3 {
                basis[row][col] = self.basis[row][0] * child.basis[0][col]
                    + self.basis[row][1] * child.basis[1][col]
                    + self.basis[row][2] * child.basis[2][col];
            }
        }
        let translation = [
            self.basis[0][0] * child.translation[0]
                + self.basis[0][1] * child.translation[1]
                + self.basis[0][2] * child.translation[2]
                + self.translation[0],
            self.basis[1][0] * child.translation[0]
                + self.basis[1][1] * child.translation[1]
                + self.basis[1][2] * child.translation[2]
                + self.translation[1],
            self.basis[2][0] * child.translation[0]
                + self.basis[2][1] * child.translation[1]
                + self.basis[2][2] * child.translation[2]
                + self.translation[2],
        ];
        Self { basis, translation }
    }

    fn to_row_major_matrix(self) -> [[f32; 4]; 4] {
        [
            [
                self.basis[0][0],
                self.basis[0][1],
                self.basis[0][2],
                self.translation[0],
            ],
            [
                self.basis[1][0],
                self.basis[1][1],
                self.basis[1][2],
                self.translation[1],
            ],
            [
                self.basis[2][0],
                self.basis[2][1],
                self.basis[2][2],
                self.translation[2],
            ],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

#[derive(Debug, Clone, Copy)]
struct ShapeTarget {
    index: usize,
    transform: ShapeTransform,
}

#[derive(Debug, Clone, Copy)]
struct ShapeAabb {
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Debug, Clone)]
struct ShapeAabbCandidates {
    aabbs: Vec<ShapeAabb>,
    ordered: bool,
}

impl ShapeAabbCandidates {
    fn ordered(aabbs: Vec<ShapeAabb>) -> Self {
        Self {
            aabbs,
            ordered: true,
        }
    }

    fn unordered(aabbs: Vec<ShapeAabb>) -> Self {
        Self {
            aabbs,
            ordered: false,
        }
    }
}

/// Decode each source body's FO76 `hknpRefMassDistribution` (COM, volume,
/// unit-mass inertia, majorAxisSpace) from a NIF collision TAG0 blob, indexed by
/// body position in `hknpPhysicsSystemData.bodyCinfos`. `None` for any body
/// without a mass distribution (statics). An empty Vec means the blob couldn't
/// be parsed — callers fall back to the AABB mass-properties approximation.
///
/// Reuses the same parse + member layout the FO76→FO4 converter relies on
/// (`convert::fo76::synthesize_motion_cinfos`); the SDK layout is documented there:
/// `centerOfMassAndVolume` (xyz=COM, w=volume), `inertiaTensor` (diagonalized,
/// unit mass), `majorAxisSpace` (quaternion).
pub fn decode_source_mass_distributions(blob: &[u8]) -> Vec<Option<SourceMassDistribution>> {
    let Ok(tagfile) = crate::hkx::parse_tagfile(blob) else {
        return Vec::new();
    };
    let Ok(hkx) = tagfile.materialize_hkx() else {
        return Vec::new();
    };
    let objects = hkx.objects();
    let Some(psd) = objects
        .iter()
        .find(|o| o.class_name == "hknpPhysicsSystemData")
    else {
        return Vec::new();
    };
    let Some(body_arr) = psd.members.iter().find(|m| m.name == "bodyCinfos") else {
        return Vec::new();
    };
    let HkxValue::Array(bodies) = &body_arr.value else {
        return Vec::new();
    };
    bodies
        .iter()
        .map(|body| {
            let members = body.as_object_members()?;
            let idx = members.iter().find_map(|bm| {
                if bm.name == "massDistribution" {
                    if let HkxValue::Pointer(Some(idx)) = bm.value {
                        return Some(idx);
                    }
                }
                None
            })?;
            read_mass_distribution(objects.get(idx)?)
        })
        .collect()
}

/// Decode source body frames from `hknpPhysicsSystemData.bodyCinfos`, indexed by
/// body position. These frames are required by articulated systems because their
/// constraint pivots are authored relative to each body's position and rotation.
pub fn decode_source_body_transforms(blob: &[u8]) -> Vec<Option<SourceBodyTransform>> {
    let Ok(hkx) = HkxFile::read(blob) else {
        return Vec::new();
    };
    let Some(psd) = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
    else {
        return Vec::new();
    };
    let Some(HkxValue::Array(bodies)) = psd
        .members
        .iter()
        .find(|member| member.name == "bodyCinfos")
        .map(|member| &member.value)
    else {
        return Vec::new();
    };

    bodies
        .iter()
        .map(|body| {
            let members = body.as_object_members()?;
            let position = vec4_member(members, "position")?;
            let orientation = vec4_member(members, "orientation")?;
            if !position.iter().all(|value| value.is_finite())
                || !orientation.iter().all(|value| value.is_finite())
            {
                return None;
            }
            Some(SourceBodyTransform {
                position,
                orientation,
            })
        })
        .collect()
}

fn vec4_member(members: &[HkxMember], name: &str) -> Option<[f32; 4]> {
    let HkxValue::F32List(values) = &members.iter().find(|member| member.name == name)?.value
    else {
        return None;
    };
    (values.len() >= 4).then(|| [values[0], values[1], values[2], values[3]])
}

fn read_mass_distribution(obj: &HkxObject) -> Option<SourceMassDistribution> {
    let mut com: Option<[f32; 4]> = None;
    let mut inertia: Option<[f32; 4]> = None;
    let mut major: Option<[f32; 4]> = None;

    fn walk(
        members: &[HkxMember],
        com: &mut Option<[f32; 4]>,
        inertia: &mut Option<[f32; 4]>,
        major: &mut Option<[f32; 4]>,
    ) {
        for m in members {
            match m.name.as_str() {
                "centerOfMassAndVolume" | "centerOfMass" => {
                    if let HkxValue::F32List(v) = &m.value {
                        if v.len() >= 4 {
                            *com = Some([v[0], v[1], v[2], v[3]]);
                        }
                    }
                }
                "inertiaTensor" => {
                    if let HkxValue::F32List(v) = &m.value {
                        if v.len() >= 4 {
                            *inertia = Some([v[0], v[1], v[2], v[3]]);
                        }
                    }
                }
                "majorAxisSpace" => {
                    if let HkxValue::F32List(v) = &m.value {
                        if v.len() == 4 {
                            *major = Some([v[0], v[1], v[2], v[3]]);
                        }
                    }
                }
                _ => {
                    if let Some(inner) = m.value.as_object_members() {
                        walk(inner, com, inertia, major);
                    }
                }
            }
        }
    }

    walk(&obj.members, &mut com, &mut inertia, &mut major);
    let com = com?;
    let inertia = inertia.unwrap_or([1.0, 1.0, 1.0, 0.0]);
    let major = major.unwrap_or([0.0, 0.0, 0.0, 1.0]);
    Some(SourceMassDistribution {
        center_of_mass: [com[0], com[1], com[2]],
        volume: com[3],
        unit_inertia: [inertia[0], inertia[1], inertia[2]],
        major_axis_space: major,
    })
}

pub fn extract_preview_meshes_from_hkx(
    hkx: &HkxFile,
    havok_scale: f32,
    body_id: Option<usize>,
) -> Vec<PreviewMesh> {
    let targets = shape_targets_for_body(hkx, body_id);
    if body_id.is_some() && targets.is_none() {
        return Vec::new();
    }
    let mut meshes = Vec::new();
    if let Some(targets) = targets {
        for target in targets {
            let mut target_meshes = preview_for_shape_index(hkx, target.index, havok_scale);
            apply_shape_transform_to_meshes(&mut target_meshes, target.transform, havok_scale);
            meshes.extend(target_meshes);
        }
        return meshes;
    }
    for idx in 0..hkx.objects().len() {
        meshes.extend(preview_for_shape_index(hkx, idx, havok_scale));
    }
    meshes
}

pub fn extract_raw_compressed_meshes_from_blob(
    blob: &[u8],
    body_id: Option<usize>,
) -> HavokResult<Vec<RawCompressedMeshData>> {
    if let Ok(hkx) = HkxFile::read(blob) {
        let mut meshes = extract_raw_compressed_meshes_from_hkx(&hkx, body_id);
        if let Ok(markers) = super::compressed_mesh::fo4_compressed_mesh_flat_convex_markers(blob) {
            if body_id.is_none() {
                for (mesh, marker) in meshes.iter_mut().zip(markers) {
                    mesh.primitive_stores_is_flat_convex = marker;
                }
            } else {
                let all_meshes = extract_raw_compressed_meshes_from_hkx(&hkx, None);
                for mesh in &mut meshes {
                    if let Some(index) = all_meshes.iter().position(|candidate| candidate == mesh) {
                        if let Some(marker) = markers.get(index) {
                            mesh.primitive_stores_is_flat_convex = *marker;
                        }
                    }
                }
            }
        }
        return Ok(meshes);
    }
    if let Ok(tagfile) = crate::hkx::parse_tagfile(blob) {
        if let Ok(hkx) = tagfile.materialize_hkx() {
            return Ok(extract_raw_compressed_meshes_from_hkx(&hkx, body_id));
        }
    }
    Ok(Vec::new())
}

pub fn extract_direct_raw_compressed_mesh_from_blob(
    blob: &[u8],
    body_id: usize,
) -> HavokResult<Option<RawCompressedMeshData>> {
    if let Ok(hkx) = HkxFile::read(blob) {
        if body_shape_class_for_body(&hkx, Some(body_id)).as_deref()
            != Some("hknpCompressedMeshShape")
        {
            return Ok(None);
        }
        return Ok(
            extract_raw_compressed_meshes_from_blob(blob, Some(body_id))?
                .into_iter()
                .next(),
        );
    }
    if let Ok(tagfile) = crate::hkx::parse_tagfile(blob) {
        if let Ok(hkx) = tagfile.materialize_hkx() {
            return Ok(extract_direct_raw_compressed_mesh_from_hkx(&hkx, body_id));
        }
    }
    Ok(None)
}

fn extract_direct_raw_compressed_mesh_from_hkx(
    hkx: &HkxFile,
    body_id: usize,
) -> Option<RawCompressedMeshData> {
    if body_shape_class_for_body(hkx, Some(body_id)).as_deref() != Some("hknpCompressedMeshShape") {
        return None;
    }
    let target = shape_targets_for_body(hkx, Some(body_id))?
        .into_iter()
        .next()?;
    raw_compressed_mesh_from_hkx(hkx, target.index)
}

pub fn extract_raw_compressed_meshes_from_hkx(
    hkx: &HkxFile,
    body_id: Option<usize>,
) -> Vec<RawCompressedMeshData> {
    let targets = shape_targets_for_body(hkx, body_id);
    if body_id.is_some() && targets.is_none() {
        return Vec::new();
    }
    let mut meshes = Vec::new();
    for (idx, obj) in hkx.objects().iter().enumerate() {
        if let Some(ref targets) = targets {
            if !targets.iter().any(|target| target.index == idx) {
                continue;
            }
        }
        if obj.class_name == "hknpCompressedMeshShape" {
            if let Some(raw) = raw_compressed_mesh_from_hkx(hkx, idx) {
                meshes.push(raw);
            }
        }
    }
    meshes
}

pub fn extract_preview_meshes_from_blob(
    blob: &[u8],
    havok_scale: f32,
    body_id: Option<usize>,
) -> HavokResult<Vec<PreviewMesh>> {
    let mut meshes = Vec::new();
    let mut body_shape_class: Option<String> = None;

    // Try packfile HKX first, then FO76/Starfield TAG0. NIF-embedded FO76
    // bhkPhysicsSystem blobs are TAG0, but materialize into the same HkxFile
    // object model as packfiles once parsed.
    if let Ok(hkx) = HkxFile::read(blob) {
        let (mut hkx_meshes, shape_class) =
            extract_preview_meshes_from_hkx_with_body_class(&hkx, havok_scale, body_id);
        if let Ok(raw_meshes) = extract_raw_compressed_meshes_from_blob(blob, body_id) {
            if raw_meshes
                .iter()
                .any(|mesh| mesh.primitive_stores_is_flat_convex == FLAT_CONVEX_ENABLED)
            {
                hkx_meshes = raw_meshes
                    .iter()
                    .filter_map(|mesh| preview_compressed_mesh_from_raw(mesh, havok_scale))
                    .collect();
            }
        }
        body_shape_class = shape_class;
        meshes.extend(hkx_meshes);
    } else if let Ok(tagfile) = crate::hkx::parse_tagfile(blob) {
        if let Ok(hkx) = tagfile.materialize_hkx() {
            let (hkx_meshes, shape_class) =
                extract_preview_meshes_from_hkx_with_body_class(&hkx, havok_scale, body_id);
            body_shape_class = shape_class;
            meshes.extend(hkx_meshes);
        }
    }

    // FO4 compressed mesh path. Run when:
    //   - body_id is None and we still have nothing (full-blob unfiltered preview), OR
    //   - body_id resolved to an `hknpCompressedMeshShape` (per-object handler can't decode it,
    //     so we must pull the section data from the raw blob).
    let try_compressed = match body_id {
        None => meshes.is_empty(),
        Some(_) => {
            meshes.is_empty() && body_shape_class.as_deref() == Some("hknpCompressedMeshShape")
        }
    };
    if try_compressed {
        if let Ok(compressed) = crate::collision::parse_fo4_compressed_mesh(blob) {
            let mut vertices: Vec<[f32; 3]> = Vec::new();
            let mut triangles: Vec<[u32; 3]> = Vec::new();
            for section in &compressed.sections {
                let offset = vertices.len() as u32;
                for &[x, y, z] in &section.vertices {
                    vertices.push([x * havok_scale, y * havok_scale, z * havok_scale]);
                }
                for &[a, b, c] in &section.triangles {
                    triangles.push([a + offset, b + offset, c + offset]);
                }
            }
            if !vertices.is_empty() && !triangles.is_empty() {
                meshes.push(PreviewMesh {
                    shape_type: "compressed_mesh".to_string(),
                    vertices,
                    triangles,
                });
            }
        }
    }

    // TAG0 fallback for raw tagfile blobs (only when nothing else matched).
    if meshes.is_empty() && body_id.is_none() {
        if let Ok(payload) = crate::collision::parse_tag0_collision_payload(blob) {
            if !payload.vertices.is_empty()
                && !payload.faces.is_empty()
                && !payload.indices.is_empty()
            {
                let mesh = mesh_from_tag0_payload(&payload, havok_scale);
                if !mesh.vertices.is_empty() {
                    meshes.push(mesh);
                }
            }
        }
    }

    Ok(meshes)
}

pub fn extract_source_polytopes_from_blob(
    blob: &[u8],
    body_id: usize,
) -> HavokResult<Vec<SourcePolytopeShape>> {
    if let Ok(hkx) = HkxFile::read(blob) {
        return Ok(extract_source_polytopes_from_hkx(&hkx, body_id));
    }
    if let Ok(tagfile) = crate::hkx::parse_tagfile(blob) {
        if let Ok(hkx) = tagfile.materialize_hkx() {
            return Ok(extract_source_polytopes_from_hkx(&hkx, body_id));
        }
    }
    Ok(Vec::new())
}

pub fn extract_direct_source_primitive_from_blob(
    blob: &[u8],
    body_id: usize,
) -> HavokResult<Option<SourcePrimitiveShape>> {
    if let Ok(hkx) = HkxFile::read(blob) {
        return Ok(direct_source_primitive_for_body(&hkx, body_id));
    }
    if let Ok(tagfile) = crate::hkx::parse_tagfile(blob) {
        if let Ok(hkx) = tagfile.materialize_hkx() {
            return Ok(direct_source_primitive_for_body(&hkx, body_id));
        }
    }
    Ok(None)
}

fn direct_source_primitive_for_body(hkx: &HkxFile, body_id: usize) -> Option<SourcePrimitiveShape> {
    let shape_index = body_shape_index_for_body(hkx, body_id)?;
    let object = hkx.objects().get(shape_index)?;
    match object.class_name.as_str() {
        "hknpSphereShape" => {
            let center = member_vec4_array4(object, "vertices").into_iter().next()?;
            let radius = member_f32(object, "convexRadius");
            if !center.iter().all(|value| value.is_finite()) || !radius.is_finite() || radius < 0.0
            {
                return None;
            }
            Some(SourcePrimitiveShape::Sphere {
                center: [center[0], center[1], center[2]],
                radius,
            })
        }
        "hknpCapsuleShape" => {
            let a = member_vec4_from_members(&object.members, "a")?;
            let b = member_vec4_from_members(&object.members, "b")?;
            let mut hull = source_polytope_from_object(object)?;
            hull.mass_properties = source_shape_mass_properties(hkx, object);
            let capsule = SourceCapsuleShape {
                a,
                b,
                convex_radius: member_f32(object, "convexRadius"),
                hull,
            };
            capsule.validate().ok()?;
            Some(SourcePrimitiveShape::Capsule(capsule))
        }
        "hknpConvexShape" => {
            let convex = SourceConvexShape {
                vertices: member_vec4_array4(object, "vertices"),
                convex_radius: member_f32(object, "convexRadius"),
                mass_properties: source_shape_mass_properties(hkx, object),
            };
            convex.validate().ok()?;
            Some(SourcePrimitiveShape::Convex(convex))
        }
        _ => None,
    }
}

pub fn extract_source_compound_children_from_blob(
    blob: &[u8],
    body_id: usize,
) -> HavokResult<Vec<CompoundChild>> {
    if let Ok(hkx) = HkxFile::read(blob) {
        return Ok(extract_source_compound_children_from_hkx(&hkx, body_id));
    }
    if let Ok(tagfile) = crate::hkx::parse_tagfile(blob) {
        if let Ok(hkx) = tagfile.materialize_hkx() {
            return Ok(extract_source_compound_children_from_hkx(&hkx, body_id));
        }
    }
    Ok(Vec::new())
}

fn extract_source_compound_children_from_hkx(hkx: &HkxFile, body_id: usize) -> Vec<CompoundChild> {
    let Some(targets) = shape_targets_for_body(hkx, Some(body_id)) else {
        return Vec::new();
    };
    targets
        .into_iter()
        .filter_map(|target| {
            let mut visiting = HashSet::new();
            let shape = source_polytope_for_shape_index(hkx, target.index, &mut visiting)?;
            Some(CompoundChild {
                transform: target.transform.to_row_major_matrix(),
                kind: CompoundChildKind::SourcePolytope { shape },
            })
        })
        .collect()
}

fn extract_source_polytopes_from_hkx(hkx: &HkxFile, body_id: usize) -> Vec<SourcePolytopeShape> {
    let Some(targets) = shape_targets_for_body(hkx, Some(body_id)) else {
        return Vec::new();
    };
    targets
        .into_iter()
        .filter_map(|target| {
            let mut visiting = HashSet::new();
            let shape = source_polytope_for_shape_index(hkx, target.index, &mut visiting)?;
            transform_source_polytope(shape, target.transform)
        })
        .collect()
}

fn source_polytope_for_shape_index(
    hkx: &HkxFile,
    shape_index: usize,
    visiting: &mut HashSet<usize>,
) -> Option<SourcePolytopeShape> {
    if !visiting.insert(shape_index) {
        return None;
    }
    let object = hkx.objects().get(shape_index)?;
    let result = match object.class_name.as_str() {
        "hknpConvexPolytopeShape" | "hkpConvexVerticesShape" | "hknpBoxShape" => {
            let mut shape = source_polytope_from_object(object);
            if let Some(shape) = shape.as_mut() {
                shape.mass_properties = source_shape_mass_properties(hkx, object);
            }
            shape
        }
        "hknpScaledConvexShape" | "hknpScaledConvexShapeBase" => {
            source_polytope_from_scaled_convex_shape(hkx, object, visiting)
        }
        _ => None,
    };
    visiting.remove(&shape_index);
    result
}

/// Decode the verbatim compressed `hknpShapeMassProperties` a source shape
/// carries via its `properties` (hkRefCountedProperties) entry. `None` when the
/// shape has no properties, no mass-props entry, or the block doesn't decode —
/// callers fall back to the zeroed static block.
fn source_shape_mass_properties(
    hkx: &HkxFile,
    shape_obj: &HkxObject,
) -> Option<super::mass_properties::CompressedMassProperties> {
    let props_index = shape_obj.members.iter().find_map(|member| {
        if member.name == "properties" {
            if let HkxValue::Pointer(Some(index)) = member.value {
                return Some(index);
            }
        }
        None
    })?;
    let refprops = hkx.objects().get(props_index)?;
    let HkxValue::Array(entries) = &refprops
        .members
        .iter()
        .find(|member| member.name == "entries")?
        .value
    else {
        return None;
    };
    for entry in entries {
        let members = entry.as_object_members()?;
        let target = members.iter().find_map(|member| {
            if member.name == "object" {
                if let HkxValue::Pointer(Some(index)) = member.value {
                    return Some(index);
                }
            }
            None
        });
        let Some(target) = target else { continue };
        let Some(object) = hkx.objects().get(target) else {
            continue;
        };
        if object.class_name != "hknpShapeMassProperties" {
            continue;
        }
        let compressed = object
            .members
            .iter()
            .find(|member| member.name == "compressedMassProperties")?;
        let fields = compressed.value.as_object_members()?;
        let center_of_mass = packed_i16x4_member(fields, "centerOfMass")?;
        let inertia = packed_i16x4_member(fields, "inertia")?;
        let major_axis_space = packed_i16x4_member(fields, "majorAxisSpace")?;
        let mass = f32_field(fields, "mass")?;
        let volume = f32_field(fields, "volume")?;
        if !mass.is_finite() || !volume.is_finite() {
            return None;
        }
        return Some(super::mass_properties::CompressedMassProperties {
            center_of_mass,
            inertia,
            major_axis_space,
            mass,
            volume,
        });
    }
    None
}

/// Read an int16x4 packed-vector member that may be either a direct array or a
/// nested struct with a `values` array (FO76 hkPackedVector serialization).
fn packed_i16x4_member(members: &[HkxMember], name: &str) -> Option<[i16; 4]> {
    let value = &members.iter().find(|member| member.name == name)?.value;
    let list = match value {
        HkxValue::Object(_) | HkxValue::TypedObject { .. } => {
            &value
                .as_object_members()?
                .iter()
                .find(|member| member.name == "values")?
                .value
        }
        other => other,
    };
    let HkxValue::Array(items) = list else {
        return None;
    };
    if items.len() < 4 {
        return None;
    }
    let mut out = [0i16; 4];
    for (slot, item) in out.iter_mut().zip(items.iter()) {
        *slot = match item {
            HkxValue::I16(v) => *v,
            HkxValue::U16(v) => *v as i16,
            HkxValue::I32(v) => i16::try_from(*v).ok()?,
            _ => return None,
        };
    }
    Some(out)
}

fn f32_field(members: &[HkxMember], name: &str) -> Option<f32> {
    match &members.iter().find(|member| member.name == name)?.value {
        HkxValue::F32(v) => Some(*v),
        _ => None,
    }
}

fn extract_preview_meshes_from_hkx_with_body_class(
    hkx: &HkxFile,
    havok_scale: f32,
    body_id: Option<usize>,
) -> (Vec<PreviewMesh>, Option<String>) {
    let body_shape_class = body_id.and_then(|_| body_shape_class_for_body(hkx, body_id));
    let meshes = extract_preview_meshes_from_hkx(hkx, havok_scale, body_id);
    (meshes, body_shape_class)
}

pub fn collision_preview_json(
    blob: &[u8],
    havok_scale: f32,
    body_id: Option<usize>,
) -> HavokResult<String> {
    let meshes = extract_preview_meshes_from_blob(blob, havok_scale, body_id)?;
    let mesh_array: Vec<serde_json::Value> = meshes
        .iter()
        .map(|mesh| {
            let vertices: Vec<serde_json::Value> = mesh
                .vertices
                .iter()
                .map(|&[x, y, z]| json!({"x": x, "y": y, "z": z}))
                .collect();
            let triangles: Vec<serde_json::Value> = mesh
                .triangles
                .iter()
                .map(|&[v1, v2, v3]| json!({"v1": v1, "v2": v2, "v3": v3}))
                .collect();
            // Nested {shape_type, mesh: {vertices, triangles}} matches the Python
            // tier-1 dict shape so callers can use a single access pattern.
            json!({
                "shape_type": mesh.shape_type,
                "mesh": {
                    "vertices": vertices,
                    "triangles": triangles,
                },
            })
        })
        .collect();
    let output = json!({ "meshes": mesh_array });
    serde_json::to_string(&output).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

// ---------------------------------------------------------------------------
// Shape dispatch
// ---------------------------------------------------------------------------

fn preview_for_shape_index(
    hkx: &HkxFile,
    shape_index: usize,
    havok_scale: f32,
) -> Vec<PreviewMesh> {
    let mut visiting = HashSet::new();
    preview_for_shape_index_inner(hkx, shape_index, havok_scale, &mut visiting)
}

fn preview_for_shape_index_inner(
    hkx: &HkxFile,
    shape_index: usize,
    havok_scale: f32,
    visiting: &mut HashSet<usize>,
) -> Vec<PreviewMesh> {
    if !visiting.insert(shape_index) {
        return Vec::new();
    }

    let meshes = match hkx.objects().get(shape_index) {
        Some(obj) if obj.class_name == "hknpCompressedMeshShape" => {
            preview_compressed_mesh_from_hkx(hkx, shape_index, havok_scale)
                .into_iter()
                .collect()
        }
        Some(obj)
            if matches!(
                obj.class_name.as_str(),
                "hknpScaledConvexShape" | "hknpScaledConvexShapeBase"
            ) =>
        {
            preview_scaled_convex_shape(hkx, obj, havok_scale, visiting)
        }
        Some(obj) => preview_for_shape_object(obj, havok_scale)
            .into_iter()
            .collect(),
        None => Vec::new(),
    };

    visiting.remove(&shape_index);
    meshes
}

fn preview_for_shape_object(obj: &HkxObject, havok_scale: f32) -> Option<PreviewMesh> {
    match obj.class_name.as_str() {
        "hknpBoxShape" | "hkpBoxShape" => preview_box(obj, havok_scale),
        "hknpSphereShape" => preview_sphere(obj, havok_scale),
        "hknpConvexShape" => preview_hknp_convex_shape(obj, havok_scale),
        "hknpCapsuleShape" => preview_capsule(obj, havok_scale),
        "hknpConvexPolytopeShape" | "hkpConvexVerticesShape" => {
            preview_convex_polytope(obj, havok_scale)
        }
        _ => None,
    }
}

fn preview_scaled_convex_shape(
    hkx: &HkxFile,
    obj: &HkxObject,
    havok_scale: f32,
    visiting: &mut HashSet<usize>,
) -> Vec<PreviewMesh> {
    let Some(core_index) = member_target_index(hkx, &obj.members, "coreShape") else {
        return Vec::new();
    };
    let scale = member_vec3(obj, "scale").unwrap_or([1.0, 1.0, 1.0]);
    let translation = member_vec3(obj, "translation").unwrap_or([0.0, 0.0, 0.0]);
    let mut meshes = preview_for_shape_index_inner(hkx, core_index, 1.0, visiting);
    let mirrored = scale[0] * scale[1] * scale[2] < 0.0;

    for mesh in &mut meshes {
        for vertex in &mut mesh.vertices {
            *vertex = [
                (translation[0] + scale[0] * vertex[0]) * havok_scale,
                (translation[1] + scale[1] * vertex[1]) * havok_scale,
                (translation[2] + scale[2] * vertex[2]) * havok_scale,
            ];
        }
        if mirrored {
            for triangle in &mut mesh.triangles {
                triangle.swap(1, 2);
            }
        }
    }

    meshes
}

fn preview_compressed_mesh_from_hkx(
    hkx: &HkxFile,
    shape_index: usize,
    havok_scale: f32,
) -> Option<PreviewMesh> {
    raw_compressed_mesh_from_hkx(hkx, shape_index)
        .and_then(|raw| preview_compressed_mesh_from_raw(&raw, havok_scale))
}

fn raw_compressed_mesh_from_hkx(
    hkx: &HkxFile,
    shape_index: usize,
) -> Option<RawCompressedMeshData> {
    let shape = hkx.objects().get(shape_index)?;
    let user_data = shape
        .members
        .iter()
        .find(|member| member.name == "userData")
        .and_then(|member| value_u64(&member.value))
        .unwrap_or(0);
    let edge_welding_map = raw_sparse_map(shape, "edgeWeldingMap").unwrap_or_default();
    let quad_is_flat = raw_bitfield(shape, "quadIsFlat").unwrap_or_default();
    let triangle_is_interior = raw_bitfield(shape, "triangleIsInterior").unwrap_or_default();
    let materials = raw_materials_for_shape(hkx, shape);
    let data_index = member_pointer(shape, "data")?;
    let data_obj = hkx.objects().get(data_index)?;
    let mesh_tree = member_object(data_obj, "meshTree")?;

    let domain = member_object_from_members(mesh_tree, "domain")?;
    let object_aabb_min = member_vec3_from_members(domain, "min")?;
    let object_aabb_max = member_vec3_from_members(domain, "max")?;
    let num_primitive_keys = member_u32(mesh_tree, "numPrimitiveKeys")?;
    let bits_per_key = member_u32(mesh_tree, "bitsPerKey")?;
    let max_key_value = member_u32(mesh_tree, "maxKeyValue")?;
    let primitive_stores_is_flat_convex = member_u32(mesh_tree, "primitiveStoresIsFlatConvex")
        .unwrap_or(0)
        .min(u8::MAX as u32) as u8;

    let master_tree_nodes = pack_master_tree_nodes(member_array(mesh_tree, "nodes")?)?;
    let section_values = member_array(mesh_tree, "sections")?;
    let primitive_values = member_array(mesh_tree, "primitives")?;
    let packed_vertex_values = member_array(mesh_tree, "packedVertices")?;
    let shared_index_values = member_array(mesh_tree, "sharedVerticesIndex").unwrap_or(&[]);
    let shared_vertex_values = member_array(mesh_tree, "sharedVertices").unwrap_or(&[]);
    let primitive_run_values = member_array(mesh_tree, "primitiveDataRuns").unwrap_or(&[]);

    let packed_vertices = packed_vertex_values
        .iter()
        .map(value_u32)
        .collect::<Option<Vec<_>>>()?;
    let shared_vertices_index = shared_index_values
        .iter()
        .map(value_u32)
        .map(|value| value.and_then(|value| u16::try_from(value).ok()))
        .collect::<Option<Vec<_>>>()?;
    let shared_vertices = shared_vertex_values
        .iter()
        .map(value_bits_u64)
        .collect::<Option<Vec<_>>>()?;
    let primitive_data_runs = primitive_run_values
        .iter()
        .map(raw_primitive_data_run)
        .collect::<Option<Vec<_>>>()?;

    let mut sections = Vec::with_capacity(section_values.len());
    for section_value in section_values {
        let section = value_object(section_value)?;
        let section_domain = member_object_from_members(section, "domain")?;
        let aabb_min = member_vec3_from_members(section_domain, "min")?;
        let aabb_max = member_vec3_from_members(section_domain, "max")?;
        let codec = member_array(section, "codecParms")?;
        if codec.len() < 6 {
            return None;
        }
        let base = [
            value_f32(&codec[0])?,
            value_f32(&codec[1])?,
            value_f32(&codec[2])?,
        ];
        let scale = [
            value_f32(&codec[3])?,
            value_f32(&codec[4])?,
            value_f32(&codec[5])?,
        ];

        let first_vertex = member_u32(section, "firstPackedVertex")? as usize;
        let num_vertices = member_u32(section, "numPackedVertices")? as usize;
        let shared_data = nested_member_u32(section, "sharedVertices", "data")?;
        let first_shared = (shared_data >> 8) as usize;
        let num_shared = member_u32(section, "numSharedIndices")? as usize;
        let primitive_data = nested_member_u32(section, "primitives", "data")?;
        let first_primitive = (primitive_data >> 8) as usize;
        let num_primitives = (primitive_data & 0xFF) as usize;
        let data_run_data = nested_member_u32(section, "dataRuns", "data")?;
        let first_data_run = (data_run_data >> 8) as usize;
        let num_data_runs = (data_run_data & 0xFF) as usize;

        let section_packed_vertices =
            packed_vertices.get(first_vertex..first_vertex + num_vertices)?;
        let section_shared_indices =
            shared_vertices_index.get(first_shared..first_shared + num_shared)?;
        let section_primitives =
            primitive_values.get(first_primitive..first_primitive + num_primitives)?;
        let section_runs =
            primitive_data_runs.get(first_data_run..first_data_run + num_data_runs)?;

        let mut primitive_bytes = Vec::with_capacity(num_primitives * 4);
        for primitive_value in section_primitives {
            let primitive = value_object(primitive_value)?;
            let indices = member_array(primitive, "indices")?;
            if indices.len() < 3 {
                return None;
            }
            let a = u8::try_from(value_u32(&indices[0])?).ok()?;
            let b = u8::try_from(value_u32(&indices[1])?).ok()?;
            let c = u8::try_from(value_u32(&indices[2])?).ok()?;
            let d = if indices.len() >= 4 {
                u8::try_from(value_u32(&indices[3])?).ok()?
            } else {
                c
            };
            primitive_bytes.extend_from_slice(&[a, b, c, d]);
        }

        sections.push(RawCompressedMeshSection {
            aabb_min,
            aabb_max,
            base,
            scale,
            packed_vertices: section_packed_vertices.to_vec(),
            shared_vertices_index: section_shared_indices.to_vec(),
            primitive_bytes,
            section_tree_nodes: pack_section_tree_nodes(member_array(section, "nodes")?)?,
            primitive_data_runs: section_runs.to_vec(),
            leaf_index: member_u32(section, "leafIndex")
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(0),
            page: member_u32(section, "page")
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0),
            flags: member_u32(section, "flags")
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0),
            layer_data: member_u32(section, "layerData")
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0),
            unused_data: member_u32(section, "unusedData")
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0),
        });
    }

    if sections.is_empty() {
        return None;
    }

    Some(RawCompressedMeshData {
        user_data,
        edge_welding_map,
        quad_is_flat,
        triangle_is_interior,
        materials,
        object_aabb_min,
        object_aabb_max,
        num_primitive_keys,
        bits_per_key,
        max_key_value,
        primitive_stores_is_flat_convex,
        master_tree_nodes,
        sections,
        shared_vertices,
    })
}

fn raw_sparse_map(shape: &HkxObject, name: &str) -> Option<RawCompressedMeshSparseMap> {
    let map = member_object(shape, name)?;
    let primary_key_to_index = member_array(map, "primaryKeyToIndex")?
        .iter()
        .map(value_u32)
        .map(|value| value.and_then(|value| u16::try_from(value).ok()))
        .collect::<Option<Vec<_>>>()?;
    let value_and_secondary_keys = member_array(map, "valueAndSecondaryKeys")?
        .iter()
        .map(value_u32)
        .map(|value| value.and_then(|value| u16::try_from(value).ok()))
        .collect::<Option<Vec<_>>>()?;
    Some(RawCompressedMeshSparseMap {
        secondary_key_mask: member_u32(map, "secondaryKeyMask").unwrap_or(u32::MAX),
        secondary_key_bits: member_u32(map, "sencondaryKeyBits")
            .or_else(|| member_u32(map, "secondaryKeyBits"))
            .unwrap_or(0),
        primary_key_to_index,
        value_and_secondary_keys,
    })
}

fn raw_bitfield(shape: &HkxObject, name: &str) -> Option<RawCompressedMeshBitField> {
    let bitfield = member_object(shape, name)?;
    let storage = member_object_from_members(bitfield, "storage")?;
    let words = member_array(storage, "words")?
        .iter()
        .map(value_u32)
        .collect::<Option<Vec<_>>>()?;
    Some(RawCompressedMeshBitField {
        words,
        num_bits: member_u32(storage, "numBits").unwrap_or(0),
    })
}

fn raw_materials_for_shape(hkx: &HkxFile, shape: &HkxObject) -> Vec<MaterialEntry> {
    let Some(properties_index) = member_pointer(shape, "properties") else {
        return Vec::new();
    };
    let Some(properties) = hkx.objects().get(properties_index) else {
        return Vec::new();
    };
    let Some(entries) = member_array(&properties.members, "entries") else {
        return Vec::new();
    };
    for entry in entries {
        let Some(entry_members) = value_object(entry) else {
            continue;
        };
        let Some(materials_index) = member_target_index(hkx, entry_members, "object") else {
            continue;
        };
        let Some(materials_object) = hkx.objects().get(materials_index) else {
            continue;
        };
        if materials_object.class_name != "hknpBSMaterialProperties" {
            continue;
        }
        let Some(values) = member_array(&materials_object.members, "MaterialA") else {
            continue;
        };
        return values
            .iter()
            .filter_map(value_object)
            .filter_map(|material| {
                Some(MaterialEntry {
                    filter_info: member_u32(material, "uiFilterInfo")?,
                    material_crc: member_u32(material, "uiMaterialCRC")?,
                })
            })
            .collect();
    }
    Vec::new()
}

fn preview_compressed_mesh_from_raw(
    raw: &RawCompressedMeshData,
    havok_scale: f32,
) -> Option<PreviewMesh> {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for section in &raw.sections {
        let idx_limit = section.packed_vertices.len() + section.shared_vertices_index.len();
        let mut local_to_vertex = vec![None; idx_limit];
        for (local_index, &packed) in section.packed_vertices.iter().enumerate() {
            let index = vertices.len() as u32;
            vertices.push(decode_packed_vertex(
                packed,
                section.base,
                section.scale,
                havok_scale,
            ));
            local_to_vertex[local_index] = Some(index);
        }
        for primitive in section.primitive_bytes.chunks_exact(4) {
            let a = primitive[0] as usize;
            let b = primitive[1] as usize;
            let c = primitive[2] as usize;
            let d = primitive[3] as usize;

            // hkcdStaticMeshTree::Primitive::getType: b==d marks a non-triangle/quad
            // primitive. FO76 (hk2015) flat-convex meshes encode their geometry as
            // CUSTOM primitives whose m_indices are [sviRecord, aabbNode, aabbNode,
            // aabbNode] (b==c==d) — NOT the hk2018 "c==CUSTOM(3)" tag convention. The
            // vertices live in the shared pool, referenced via the sviRecord header,
            // so the triangle path can't resolve them and the mesh would otherwise
            // drop to a coarse AABB box. Decode + convex-hull-triangulate each custom
            // primitive: m_indices[0] is the sharedVerticesIndex record, m_indices[1]
            // the section-tree AABB node. The decoder validates the record header and
            // returns without emitting anything for non-custom b==d primitives.
            if b == d {
                if raw.primitive_stores_is_flat_convex == FLAT_CONVEX_ENABLED {
                    decode_custom_flat_convex_primitive(
                        raw,
                        section,
                        a,
                        b,
                        havok_scale,
                        &mut vertices,
                        &mut triangles,
                    );
                }
                continue;
            }

            let Some(va) = compressed_section_vertex_index(
                raw,
                section,
                a,
                &mut local_to_vertex,
                &mut vertices,
                havok_scale,
            ) else {
                continue;
            };
            let Some(vb) = compressed_section_vertex_index(
                raw,
                section,
                b,
                &mut local_to_vertex,
                &mut vertices,
                havok_scale,
            ) else {
                continue;
            };
            let Some(vc) = compressed_section_vertex_index(
                raw,
                section,
                c,
                &mut local_to_vertex,
                &mut vertices,
                havok_scale,
            ) else {
                continue;
            };
            if c != d {
                let Some(vd) = compressed_section_vertex_index(
                    raw,
                    section,
                    d,
                    &mut local_to_vertex,
                    &mut vertices,
                    havok_scale,
                ) else {
                    continue;
                };
                triangles.push([va, vb, vc]);
                triangles.push([va, vc, vd]);
            } else {
                triangles.push([va, vb, vc]);
            }
        }
    }
    if vertices.is_empty() || triangles.is_empty() {
        return None;
    }
    Some(PreviewMesh {
        shape_type: "compressed_mesh".to_string(),
        vertices,
        triangles,
    })
}

/// `hknpCompressedMeshShapeData::meshTree::primitiveStoresIsFlatConvex` sentinel
/// (0xFF) — the mesh stores flat-convex CUSTOM primitives in the shared pool.
const FLAT_CONVEX_ENABLED: u8 = 0xFF;

/// Decode one CUSTOM flat-convex primitive and append its preview triangles
/// to the running preview mesh.
///
/// Mirrors `hkcdStaticMeshTree::SectionDecoder::getCustomPrimitiveVertices`:
/// `record_ref` is `m_indices[0]` in section vertex-index space (`[0, numPacked)`
/// packed, `[numPacked, …)` shared), so subtract the packed count to reach the
/// `sharedVerticesIndex` slot — the same offset the triangle path applies. That
/// slot packs the record header (numVertices<<8 | numTags<<6 | compression<<4 |
/// type); the next slot is `firstShared` — the start element in the u64 shared
/// pool. For the LOCAL codecs the quantization AABB is the section-tree node
/// `aabb_node` (= the primitive's `m_indices[1]`, a direct node index).
fn decode_custom_flat_convex_primitive(
    raw: &RawCompressedMeshData,
    section: &RawCompressedMeshSection,
    record_ref: usize,
    aabb_node: usize,
    havok_scale: f32,
    vertices: &mut Vec<[f32; 3]>,
    triangles: &mut Vec<[u32; 3]>,
) -> Option<()> {
    // Without this offset the header is read from the wrong slot (or out of
    // bounds → the primitive silently drops), which left ~half of every
    // flat-convex SCOL hull uncollided and players fell through.
    let header_idx = record_ref.checked_sub(section.packed_vertices.len())?;
    let header = *section.shared_vertices_index.get(header_idx)? as u32;
    let primitive_type = header & 0xf;
    let num_tags = ((header >> 6) & 0x3) as usize;
    let num_vertices = (header >> 8) as usize;
    let compression = (header >> 4) & 0x3;
    let first_shared = *section.shared_vertices_index.get(header_idx + 1)? as usize;
    let tag_start = header_idx + 2;

    let local = match compression {
        // GLOBAL: 21-21-22 against the object domain, read sequentially.
        0 => decode_custom_vertices(
            &raw.shared_vertices,
            section.page as u32,
            first_shared,
            num_vertices,
            raw.object_aabb_min,
            raw.object_aabb_max,
            64,
        )?,
        // LOCAL_4: 11-11-10 against the section-tree node AABB (u32 words).
        1 => {
            let (amin, amax) = section_node_aabb(section, aabb_node)?;
            decode_custom_vertices(
                &raw.shared_vertices,
                section.page as u32,
                first_shared,
                num_vertices,
                amin,
                amax,
                32,
            )?
        }
        // LOCAL_2: 5-5-6 against the section-tree node AABB (u16 words).
        2 => {
            let (amin, amax) = section_node_aabb(section, aabb_node)?;
            decode_custom_vertices(
                &raw.shared_vertices,
                section.page as u32,
                first_shared,
                num_vertices,
                amin,
                amax,
                16,
            )?
        }
        _ => return None,
    };

    let base = vertices.len() as u32;
    match primitive_type {
        CUSTOM_PRIMITIVE_SPHERE => {
            let center = *local.first()?;
            let radius = custom_primitive_radius(section, tag_start, num_tags)?;
            if radius <= 0.0 {
                return None;
            }
            let (sphere_vertices, sphere_triangles) =
                sphere_mesh(center, radius, havok_scale, 12, 8);
            vertices.extend_from_slice(&sphere_vertices);
            triangles.extend(
                sphere_triangles
                    .into_iter()
                    .map(|tri| [base + tri[0], base + tri[1], base + tri[2]]),
            );
            return Some(());
        }
        CUSTOM_PRIMITIVE_CAPSULE => {
            if local.len() < 2 {
                return None;
            }
            let radius = custom_primitive_radius(section, tag_start, num_tags)?;
            if radius <= 0.0 {
                return None;
            }
            let (capsule_vertices, capsule_triangles) =
                capsule_mesh(local[0], local[1], radius, havok_scale, 12, 4);
            vertices.extend_from_slice(&capsule_vertices);
            triangles.extend(
                capsule_triangles
                    .into_iter()
                    .map(|tri| [base + tri[0], base + tri[1], base + tri[2]]),
            );
            return Some(());
        }
        CUSTOM_PRIMITIVE_CONVEX | LEGACY_CUSTOM_PRIMITIVE_CONVEX => {}
        _ => return None,
    }

    if num_vertices < 3 {
        return None;
    }

    let scaled: Vec<[f32; 3]> = local
        .iter()
        .map(|v| [v[0] * havok_scale, v[1] * havok_scale, v[2] * havok_scale])
        .collect();

    if let Ok(topo) = crate::collision::hull::compute_hull_topology_robust(&scaled) {
        if !topo.vertices.is_empty() {
            let faces_u32: Vec<(u32, u32, u32)> = topo
                .faces
                .iter()
                .map(|&(first, num, half)| (first as u32, num as u32, half as u32))
                .collect();
            vertices.extend_from_slice(&topo.vertices);
            for tri in triangles_from_faces(&faces_u32, &topo.indices) {
                triangles.push([base + tri[0], base + tri[1], base + tri[2]]);
            }
            return Some(());
        }
    }

    let (flat_vertices, flat_triangles) = triangulate_coplanar_convex_vertices(&scaled)?;
    vertices.extend_from_slice(&flat_vertices);
    for tri in flat_triangles {
        triangles.push([base + tri[0], base + tri[1], base + tri[2]]);
    }
    Some(())
}

const CUSTOM_PRIMITIVE_SPHERE: u32 = 0;
const CUSTOM_PRIMITIVE_CAPSULE: u32 = 1;
const CUSTOM_PRIMITIVE_CONVEX: u32 = 2;
const LEGACY_CUSTOM_PRIMITIVE_CONVEX: u32 = 3;

fn custom_primitive_radius(
    section: &RawCompressedMeshSection,
    tag_start: usize,
    num_tags: usize,
) -> Option<f32> {
    if num_tags == 0 {
        return None;
    }
    let encoded = *section.shared_vertices_index.get(tag_start)?;
    Some(decode_hknp_i16_float(encoded))
}

fn decode_hknp_i16_float(value: u16) -> f32 {
    let signed = value as i16 as i32;
    f32::from_bits((signed << 16) as u32)
}

fn triangulate_coplanar_convex_vertices(
    vertices: &[[f32; 3]],
) -> Option<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
    let mut unique = Vec::new();
    for &vertex in vertices {
        if !vertex.iter().all(|value| value.is_finite()) {
            return None;
        }
        if unique
            .iter()
            .all(|&existing| distance_squared(vertex, existing) > 1.0e-8)
        {
            unique.push(vertex);
        }
    }
    if unique.len() < 3 {
        return None;
    }

    let origin = unique[0];
    let mut normal = [0.0; 3];
    let mut normal_len_sq = 0.0;
    for i in 1..unique.len() {
        let a = sub3(unique[i], origin);
        for &candidate in unique.iter().skip(i + 1) {
            let b = sub3(candidate, origin);
            let cross = cross(a, b);
            let len_sq = dot3(cross, cross);
            if len_sq > normal_len_sq {
                normal = cross;
                normal_len_sq = len_sq;
            }
        }
    }
    if normal_len_sq <= 1.0e-10 {
        return None;
    }

    let normal = normalize_vector(normal);
    let (basis_u, basis_v) = orthonormal_basis(normal);
    let center = average_xyz(&unique);
    unique.sort_by(|a, b| {
        let da = sub3(*a, center);
        let db = sub3(*b, center);
        let aa = dot3(da, basis_v).atan2(dot3(da, basis_u));
        let ab = dot3(db, basis_v).atan2(dot3(db, basis_u));
        aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
    });

    let triangles = fan_triangles(unique.len());
    if triangles.is_empty() {
        return None;
    }
    Some((unique, triangles))
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub3(a, b);
    dot3(d, d)
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// hkcdStaticMeshTree shared-vertex page size (hkcdStaticMeshTree.h:34).
const SHARED_VERTICES_PAGE_SIZE: usize = 65536;

/// Absolute index into the shared-vertex pool for a section's local shared index.
/// Shared indices are relative to `page * SHARED_VERTICES_PAGE_SIZE`
/// (hkcdStaticMeshTreeDecoder.inl:41).
fn shared_pool_index(page: u32, local_index: usize) -> usize {
    page as usize * SHARED_VERTICES_PAGE_SIZE + local_index
}

/// Decode `num_vertices` from the u64 shared pool starting at element
/// `first_shared` (relative to `page`), interpreting it as `type_bits`-wide
/// words with the Havok little-endian `fetchIndex` word-swap, dequantizing
/// against `[amin, amax]`.
///
/// `type_bits` selects the codec width: 32 = LOCAL_4 (11-11-10),
/// 16 = LOCAL_2 (5-5-6), 64 = GLOBAL/shared (21-21-22).
fn decode_custom_vertices(
    pool: &[u64],
    page: u32,
    first_shared: usize,
    num_vertices: usize,
    amin: [f32; 3],
    amax: [f32; 3],
    type_bits: u32,
) -> Option<Vec<[f32; 3]>> {
    // hkcdStaticMeshTree VertexCodec::getBitsForType: x == y, z = remainder.
    let bits_x = (type_bits + 1) / 3;
    let bits_z = type_bits - 2 * bits_x;
    let mask_x = (1u64 << bits_x) - 1;
    let mask_z = (1u64 << bits_z) - 1;
    let sx = scale_for(amin[0], amax[0], mask_x);
    let sy = scale_for(amin[1], amax[1], mask_x);
    let sz = scale_for(amin[2], amax[2], mask_z);
    // fetchIndex swaps order within each u64 (sizeof(u64)/sizeof(word) words).
    let words_per_u64 = (64 / type_bits) as usize;
    let m = words_per_u64.saturating_sub(1);
    let word_mask: u64 = if type_bits >= 64 {
        u64::MAX
    } else {
        (1u64 << type_bits) - 1
    };

    let page_base = shared_pool_index(page, first_shared);
    let mut out = Vec::with_capacity(num_vertices);
    for j in 0..num_vertices {
        let fetch = (j & !m) + (m - (j & m));
        let bit_off = fetch as u64 * type_bits as u64;
        let word_idx = page_base + (bit_off / 64) as usize;
        let raw_word = *pool.get(word_idx)?;
        let shift = (bit_off % 64) as u32;
        let value = (raw_word >> shift) & word_mask;
        let qx = value & mask_x;
        let qy = (value >> bits_x) & mask_x;
        let qz = (value >> (2 * bits_x)) & mask_z;
        out.push([
            amin[0] + qx as f32 * sx,
            amin[1] + qy as f32 * sy,
            amin[2] + qz as f32 * sz,
        ]);
    }
    Some(out)
}

fn scale_for(min: f32, max: f32, mask: u64) -> f32 {
    if max > min && mask > 0 {
        (max - min) / mask as f32
    } else {
        0.0
    }
}

/// Walk the section's Aabb4 tree from the root to `target`, returning its decoded
/// AABB. Mirrors `hkcdStaticTree::AabbTree::getNodeAabb` with the Aabb4BytesCodec
/// unpack (4-bit squared-fraction nibbles, extent = parent-extent / 226).
fn section_node_aabb(
    section: &RawCompressedMeshSection,
    target: usize,
) -> Option<([f32; 3], [f32; 3])> {
    let nodes = &section.section_tree_nodes;
    let node_count = nodes.len() / 4;
    if target >= node_count {
        return None;
    }
    let mut amin = section.aabb_min;
    let mut amax = section.aabb_max;
    let mut current = 0usize;
    let mut guard = 0usize;
    while current != target {
        let data = nodes[current * 4 + 3];
        if data & 1 == 0 {
            // Reached a leaf before the target node — malformed tree.
            return None;
        }
        let delta = (data & 0xfe) as usize;
        let left = current + 1;
        let right = current + delta;
        let child = if target >= right { right } else { left };
        if child == current || child >= node_count {
            return None;
        }
        unpack_child_aabb(&mut amin, &mut amax, &nodes[child * 4..child * 4 + 3]);
        current = child;
        guard += 1;
        if guard > node_count {
            return None;
        }
    }
    Some((amin, amax))
}

fn unpack_child_aabb(amin: &mut [f32; 3], amax: &mut [f32; 3], xyz: &[u8]) {
    let ext = [
        (amax[0] - amin[0]) / 226.0,
        (amax[1] - amin[1]) / 226.0,
        (amax[2] - amin[2]) / 226.0,
    ];
    for a in 0..3 {
        let min_nib = (xyz[a] >> 4) as f32;
        let max_nib = (xyz[a] & 0xf) as f32;
        amin[a] += ext[a] * min_nib * min_nib;
        amax[a] -= ext[a] * max_nib * max_nib;
    }
}

fn compressed_section_vertex_index(
    raw: &RawCompressedMeshData,
    section: &RawCompressedMeshSection,
    local_index: usize,
    local_to_vertex: &mut [Option<u32>],
    vertices: &mut Vec<[f32; 3]>,
    havok_scale: f32,
) -> Option<u32> {
    if local_index >= local_to_vertex.len() {
        return None;
    }
    if let Some(index) = local_to_vertex[local_index] {
        return Some(index);
    }
    let shared_local = local_index.checked_sub(section.packed_vertices.len())?;
    let shared_index = *section.shared_vertices_index.get(shared_local)? as usize;
    let shared = *raw
        .shared_vertices
        .get(shared_pool_index(section.page as u32, shared_index))?;
    let index = vertices.len() as u32;
    vertices.push(decode_shared_vertex(
        shared,
        raw.object_aabb_min,
        raw.object_aabb_max,
        havok_scale,
    ));
    local_to_vertex[local_index] = Some(index);
    Some(index)
}

fn decode_packed_vertex(
    packed: u32,
    base: [f32; 3],
    scale: [f32; 3],
    havok_scale: f32,
) -> [f32; 3] {
    let x = packed & 0x7FF;
    let y = (packed >> 11) & 0x7FF;
    let z = (packed >> 22) & 0x3FF;
    [
        (base[0] + x as f32 * scale[0]) * havok_scale,
        (base[1] + y as f32 * scale[1]) * havok_scale,
        (base[2] + z as f32 * scale[2]) * havok_scale,
    ]
}

fn decode_shared_vertex(
    packed: u64,
    object_aabb_min: [f32; 3],
    object_aabb_max: [f32; 3],
    havok_scale: f32,
) -> [f32; 3] {
    let qx = packed & 0x1F_FFFF;
    let qy = (packed >> 21) & 0x1F_FFFF;
    let qz = (packed >> 42) & 0x3F_FFFF;
    let sx = if object_aabb_max[0] > object_aabb_min[0] {
        (object_aabb_max[0] - object_aabb_min[0]) / ((1u64 << 21) - 1) as f32
    } else {
        0.0
    };
    let sy = if object_aabb_max[1] > object_aabb_min[1] {
        (object_aabb_max[1] - object_aabb_min[1]) / ((1u64 << 21) - 1) as f32
    } else {
        0.0
    };
    let sz = if object_aabb_max[2] > object_aabb_min[2] {
        (object_aabb_max[2] - object_aabb_min[2]) / ((1u64 << 22) - 1) as f32
    } else {
        0.0
    };
    [
        (object_aabb_min[0] + qx as f32 * sx) * havok_scale,
        (object_aabb_min[1] + qy as f32 * sy) * havok_scale,
        (object_aabb_min[2] + qz as f32 * sz) * havok_scale,
    ]
}

fn pack_master_tree_nodes(values: &[HkxValue]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(values.len() * 5);
    for value in values {
        let node = value_object(value)?;
        let xyz = member_array(node, "xyz")?;
        if xyz.len() < 3 {
            return None;
        }
        bytes.push(u8::try_from(value_u32(&xyz[0])?).ok()?);
        bytes.push(u8::try_from(value_u32(&xyz[1])?).ok()?);
        bytes.push(u8::try_from(value_u32(&xyz[2])?).ok()?);
        let hi = u8::try_from(member_u32(node, "hiData")?).ok()?;
        let lo = u8::try_from(member_u32(node, "loData")?).ok()?;
        bytes.push(hi);
        bytes.push(lo);
    }
    Some(bytes)
}

fn pack_section_tree_nodes(values: &[HkxValue]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        let node = value_object(value)?;
        let xyz = member_array(node, "xyz")?;
        if xyz.len() < 3 {
            return None;
        }
        bytes.push(u8::try_from(value_u32(&xyz[0])?).ok()?);
        bytes.push(u8::try_from(value_u32(&xyz[1])?).ok()?);
        bytes.push(u8::try_from(value_u32(&xyz[2])?).ok()?);
        bytes.push(u8::try_from(member_u32(node, "data")?).ok()?);
    }
    Some(bytes)
}

fn raw_primitive_data_run(value: &HkxValue) -> Option<RawCompressedMeshDataRun> {
    let run = value_object(value)?;
    Some(RawCompressedMeshDataRun {
        value: u16::try_from(nested_member_u32(run, "value", "data")?).ok()?,
        index: u8::try_from(member_u32(run, "index")?).ok()?,
        count: u8::try_from(member_u32(run, "count")?).ok()?,
    })
}

fn preview_box(obj: &HkxObject, havok_scale: f32) -> Option<PreviewMesh> {
    if let Some(mut mesh) = preview_convex_polytope(obj, havok_scale) {
        mesh.shape_type = "box".to_string();
        return Some(mesh);
    }
    let half_extents =
        member_vec3(obj, "halfExtents").or_else(|| member_vec3(obj, "halfExtentsAndRadius"))?;
    let vertices = box_vertices(half_extents, havok_scale);
    let triangles = box_triangles();
    Some(PreviewMesh {
        shape_type: "box".to_string(),
        vertices,
        triangles,
    })
}

fn preview_sphere(obj: &HkxObject, havok_scale: f32) -> Option<PreviewMesh> {
    let radius = member_f32(obj, "convexRadius");
    if radius <= 0.0 {
        return None;
    }
    let verts_raw = member_vec4_array(obj, "vertices");
    let center = if verts_raw.is_empty() {
        [0.0_f32, 0.0, 0.0]
    } else {
        average_xyz(&verts_raw)
    };
    let (vertices, triangles) = sphere_mesh(center, radius, havok_scale, 12, 8);
    Some(PreviewMesh {
        shape_type: "sphere".to_string(),
        vertices,
        triangles,
    })
}

fn preview_hknp_convex_shape(obj: &HkxObject, havok_scale: f32) -> Option<PreviewMesh> {
    let raw_verts = member_vec4_array(obj, "vertices");
    if !raw_verts.is_empty() {
        let vertices: Vec<[f32; 3]> = raw_verts
            .iter()
            .map(|&[x, y, z]| [x * havok_scale, y * havok_scale, z * havok_scale])
            .collect();
        let triangles = fan_triangles(vertices.len());
        if triangles.is_empty() {
            return None;
        }
        return Some(PreviewMesh {
            shape_type: "convex_hull".to_string(),
            vertices,
            triangles,
        });
    }

    preview_sphere(obj, havok_scale)
}

fn preview_capsule(obj: &HkxObject, havok_scale: f32) -> Option<PreviewMesh> {
    let a = member_vec4_from_members(&obj.members, "a")?;
    let b = member_vec4_from_members(&obj.members, "b")?;
    let point_a = [a[0], a[1], a[2]];
    let point_b = [b[0], b[1], b[2]];
    let radius = if a[3].is_finite() && a[3] > 0.0 && (a[3] - 1.0).abs() > 1e-4 {
        a[3]
    } else {
        member_f32(obj, "convexRadius")
    };
    if radius <= 0.0 {
        return None;
    }
    let (vertices, triangles) = capsule_mesh(point_a, point_b, radius, havok_scale, 12, 4);
    Some(PreviewMesh {
        shape_type: "capsule".to_string(),
        vertices,
        triangles,
    })
}

fn preview_convex_polytope(obj: &HkxObject, havok_scale: f32) -> Option<PreviewMesh> {
    let raw_verts = member_vec4_array(obj, "vertices");
    if raw_verts.is_empty() {
        return None;
    }
    let vertices: Vec<[f32; 3]> = raw_verts
        .iter()
        .map(|&[x, y, z]| [x * havok_scale, y * havok_scale, z * havok_scale])
        .collect();
    let faces = face_tuples_member(obj, "faces");
    let indices = uint8_array_member(obj, "indices");
    let triangles = if !faces.is_empty() && !indices.is_empty() {
        triangles_from_faces(&faces, &indices)
    } else {
        fan_triangles(vertices.len())
    };
    Some(PreviewMesh {
        shape_type: "convex_hull".to_string(),
        vertices,
        triangles,
    })
}

fn source_polytope_from_object(obj: &HkxObject) -> Option<SourcePolytopeShape> {
    let vertices = member_vec4_array(obj, "vertices");
    let planes = member_vec4_array4(obj, "planes");
    let faces = face_tuples_member(obj, "faces")
        .into_iter()
        .map(|(first, count, angle)| {
            Some((
                u16::try_from(first).ok()?,
                u8::try_from(count).ok()?,
                u8::try_from(angle).ok()?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    let indices = uint8_array_member(obj, "indices");
    let shape = SourcePolytopeShape {
        vertices,
        planes,
        faces,
        indices,
        convex_radius: member_f32(obj, "convexRadius"),
        mass_properties: None,
    };
    shape.validate().ok()?;
    Some(shape)
}

fn source_polytope_from_scaled_convex_shape(
    hkx: &HkxFile,
    obj: &HkxObject,
    visiting: &mut HashSet<usize>,
) -> Option<SourcePolytopeShape> {
    let core_index = member_target_index(hkx, &obj.members, "coreShape")?;
    let shape = source_polytope_for_shape_index(hkx, core_index, visiting)?;
    let scale = member_vec3(obj, "scale").unwrap_or([1.0, 1.0, 1.0]);
    let translation = member_vec3(obj, "translation").unwrap_or([0.0, 0.0, 0.0]);
    let transform = ShapeTransform {
        basis: [
            [scale[0], 0.0, 0.0],
            [0.0, scale[1], 0.0],
            [0.0, 0.0, scale[2]],
        ],
        translation,
    };
    transform_source_polytope(shape, transform)
}

fn transform_source_polytope(
    mut shape: SourcePolytopeShape,
    transform: ShapeTransform,
) -> Option<SourcePolytopeShape> {
    if transform.is_identity() {
        return Some(shape);
    }

    let det = determinant3(transform.basis);
    if !det.is_finite() || det.abs() < 1e-8 {
        return None;
    }
    let inverse = inverse3(transform.basis)?;

    for vertex in &mut shape.vertices {
        *vertex = transform_point(transform, *vertex);
        if !vertex.iter().all(|value| value.is_finite()) {
            return None;
        }
    }

    for plane in &mut shape.planes {
        *plane = transform_plane(inverse, transform.translation, *plane)?;
    }

    if det < 0.0 {
        reverse_source_polytope_face_winding(&mut shape);
    }

    let max_scale = transform
        .basis
        .iter()
        .map(|row| (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt())
        .fold(0.0_f32, f32::max);
    if max_scale.is_finite() && max_scale > 0.0 {
        shape.convex_radius *= max_scale;
    }

    shape.validate().ok()?;
    Some(shape)
}

fn transform_point(transform: ShapeTransform, point: [f32; 3]) -> [f32; 3] {
    [
        transform.basis[0][0] * point[0]
            + transform.basis[0][1] * point[1]
            + transform.basis[0][2] * point[2]
            + transform.translation[0],
        transform.basis[1][0] * point[0]
            + transform.basis[1][1] * point[1]
            + transform.basis[1][2] * point[2]
            + transform.translation[1],
        transform.basis[2][0] * point[0]
            + transform.basis[2][1] * point[1]
            + transform.basis[2][2] * point[2]
            + transform.translation[2],
    ]
}

fn transform_plane(
    inverse_basis: [[f32; 3]; 3],
    translation: [f32; 3],
    plane: [f32; 4],
) -> Option<[f32; 4]> {
    let normal = [plane[0], plane[1], plane[2]];
    let transformed_normal = [
        inverse_basis[0][0] * normal[0]
            + inverse_basis[1][0] * normal[1]
            + inverse_basis[2][0] * normal[2],
        inverse_basis[0][1] * normal[0]
            + inverse_basis[1][1] * normal[1]
            + inverse_basis[2][1] * normal[2],
        inverse_basis[0][2] * normal[0]
            + inverse_basis[1][2] * normal[1]
            + inverse_basis[2][2] * normal[2],
    ];
    let transformed_offset = plane[3]
        - (transformed_normal[0] * translation[0]
            + transformed_normal[1] * translation[1]
            + transformed_normal[2] * translation[2]);
    let length = (transformed_normal[0] * transformed_normal[0]
        + transformed_normal[1] * transformed_normal[1]
        + transformed_normal[2] * transformed_normal[2])
        .sqrt();
    if !length.is_finite() || length < 1e-8 {
        return None;
    }
    Some([
        transformed_normal[0] / length,
        transformed_normal[1] / length,
        transformed_normal[2] / length,
        transformed_offset / length,
    ])
}

fn reverse_source_polytope_face_winding(shape: &mut SourcePolytopeShape) {
    for &(first, count, _) in &shape.faces {
        let first = usize::from(first);
        let count = usize::from(count);
        let end = first.saturating_add(count);
        if end <= shape.indices.len() {
            shape.indices[first..end].reverse();
        }
    }
}

// ---------------------------------------------------------------------------
// TAG0 payload → mesh (fallback path for raw tagfile blobs)
// ---------------------------------------------------------------------------

fn mesh_from_tag0_payload(
    payload: &crate::collision::Tag0CollisionPayload,
    havok_scale: f32,
) -> PreviewMesh {
    let vertices: Vec<[f32; 3]> = payload
        .vertices
        .iter()
        .map(|&[x, y, z]| [x * havok_scale, y * havok_scale, z * havok_scale])
        .collect();
    // Convert faces (first_index: u16, num_indices: u8, min_half_angle: u8) + byte indices.
    let face_tuples: Vec<(usize, usize, usize)> = payload
        .faces
        .iter()
        .map(|&(first, num, angle)| (first as usize, num as usize, angle as usize))
        .collect();
    let indices_usize: Vec<usize> = payload.indices.iter().map(|&b| b as usize).collect();
    let triangles = triangles_from_faces_usize(&face_tuples, &indices_usize);
    PreviewMesh {
        shape_type: "convex_hull".to_string(),
        vertices,
        triangles,
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

fn box_vertices(half: [f32; 3], scale: f32) -> Vec<[f32; 3]> {
    let hx = half[0] * scale;
    let hy = half[1] * scale;
    let hz = half[2] * scale;
    vec![
        [-hx, -hy, -hz],
        [hx, -hy, -hz],
        [hx, hy, -hz],
        [-hx, hy, -hz],
        [-hx, -hy, hz],
        [hx, -hy, hz],
        [hx, hy, hz],
        [-hx, hy, hz],
    ]
}

fn box_triangles() -> Vec<[u32; 3]> {
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

fn sphere_mesh(
    center: [f32; 3],
    radius: f32,
    scale: f32,
    segments: usize,
    rings: usize,
) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let cx = center[0] * scale;
    let cy = center[1] * scale;
    let cz = center[2] * scale;
    let r = radius * scale;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for ring in 0..=rings {
        let theta = std::f32::consts::PI * ring as f32 / rings as f32;
        let sin_t = theta.sin();
        let cos_t = theta.cos();
        for seg in 0..segments {
            let phi = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
            vertices.push([
                cx + r * sin_t * phi.cos(),
                cy + r * sin_t * phi.sin(),
                cz + r * cos_t,
            ]);
        }
    }
    for ring in 0..rings {
        for seg in 0..segments {
            let next_seg = (seg + 1) % segments;
            let current = (ring * segments + seg) as u32;
            let next_row = ((ring + 1) * segments + seg) as u32;
            let current_next = (ring * segments + next_seg) as u32;
            let next_row_next = ((ring + 1) * segments + next_seg) as u32;
            triangles.push([current, next_row, current_next]);
            triangles.push([current_next, next_row, next_row_next]);
        }
    }
    (vertices, triangles)
}

fn capsule_mesh(
    point_a: [f32; 3],
    point_b: [f32; 3],
    radius: f32,
    scale: f32,
    segments: usize,
    hemi_rings: usize,
) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let ax = point_a[0] * scale;
    let ay = point_a[1] * scale;
    let az = point_a[2] * scale;
    let bx = point_b[0] * scale;
    let by = point_b[1] * scale;
    let bz = point_b[2] * scale;
    let r = radius * scale;
    let direction = [bx - ax, by - ay, bz - az];
    let (basis_u, basis_v) = orthonormal_basis(direction);
    let axis = normalize_vector(direction);

    let mut vertices: Vec<[f32; 3]> = Vec::new();
    for ring in 0..=hemi_rings {
        let angle = std::f32::consts::FRAC_PI_2 * ring as f32 / hemi_rings as f32;
        let radial = angle.sin() * r;
        let offset = angle.cos() * r;
        for seg in 0..segments {
            let phi = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
            let dx = phi.cos() * basis_u[0] + phi.sin() * basis_v[0];
            let dy = phi.cos() * basis_u[1] + phi.sin() * basis_v[1];
            let dz = phi.cos() * basis_u[2] + phi.sin() * basis_v[2];
            // Cap at A (subtract offset from A along axis)
            vertices.push([
                ax - axis[0] * offset + dx * radial,
                ay - axis[1] * offset + dy * radial,
                az - axis[2] * offset + dz * radial,
            ]);
            // Cap at B (add offset to B along axis)
            vertices.push([
                bx + axis[0] * offset + dx * radial,
                by + axis[1] * offset + dy * radial,
                bz + axis[2] * offset + dz * radial,
            ]);
        }
    }
    let triangles = fan_triangles(vertices.len());
    (vertices, triangles)
}

fn triangles_from_faces(faces: &[(u32, u32, u32)], indices: &[u8]) -> Vec<[u32; 3]> {
    let usize_faces: Vec<(usize, usize, usize)> = faces
        .iter()
        .map(|&(a, b, c)| (a as usize, b as usize, c as usize))
        .collect();
    let usize_indices: Vec<usize> = indices.iter().map(|&b| b as usize).collect();
    triangles_from_faces_usize(&usize_faces, &usize_indices)
}

fn triangles_from_faces_usize(faces: &[(usize, usize, usize)], indices: &[usize]) -> Vec<[u32; 3]> {
    let mut triangles = Vec::new();
    for &(first_index, num_indices, _) in faces {
        if num_indices < 3 || first_index >= indices.len() {
            continue;
        }
        let Some(end) = first_index
            .checked_add(num_indices)
            .map(|value| value.min(indices.len()))
        else {
            continue;
        };
        let face_indices = &indices[first_index..end];
        if face_indices.len() < 3 {
            continue;
        }
        let anchor = face_indices[0] as u32;
        for i in 1..face_indices.len() - 1 {
            triangles.push([anchor, face_indices[i] as u32, face_indices[i + 1] as u32]);
        }
    }
    triangles
}

fn apply_shape_transform_to_meshes(
    meshes: &mut [PreviewMesh],
    transform: ShapeTransform,
    havok_scale: f32,
) {
    if transform.is_identity() {
        return;
    }
    let mirrored = determinant3(transform.basis) < 0.0;
    for mesh in meshes {
        for vertex in &mut mesh.vertices {
            let [x, y, z] = *vertex;
            *vertex = [
                transform.basis[0][0] * x
                    + transform.basis[0][1] * y
                    + transform.basis[0][2] * z
                    + transform.translation[0] * havok_scale,
                transform.basis[1][0] * x
                    + transform.basis[1][1] * y
                    + transform.basis[1][2] * z
                    + transform.translation[1] * havok_scale,
                transform.basis[2][0] * x
                    + transform.basis[2][1] * y
                    + transform.basis[2][2] * z
                    + transform.translation[2] * havok_scale,
            ];
        }
        if mirrored {
            for triangle in &mut mesh.triangles {
                triangle.swap(1, 2);
            }
        }
    }
}

fn determinant3(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

fn inverse3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let det = determinant3(m);
    if !det.is_finite() || det.abs() < 1e-8 {
        return None;
    }
    let inv_det = 1.0 / det;
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        RawCompressedMeshBitField, RawCompressedMeshData, RawCompressedMeshSection,
        RawCompressedMeshSparseMap, ShapeTransform, SourcePrimitiveShape,
        direct_source_primitive_for_body, extract_source_polytopes_from_hkx,
        preview_compressed_mesh_from_raw, shared_pool_index, transform_source_polytope,
        triangles_from_faces_usize,
    };
    use crate::collision::compressed_mesh::{pack_vertex_11_11_10, pack_vertex_21_21_22};
    use crate::collision::polytope::SourcePolytopeShape;
    use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
    use crate::hkx::types::HkxValue;

    fn direct_body_file(shape: HkxObject) -> HkxFile {
        let physics_system = HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hknpPhysicsSystemData".to_string(),
            members: vec![HkxMember {
                name: "bodyCinfos".to_string(),
                value: HkxValue::Array(vec![HkxValue::Object(vec![HkxMember {
                    name: "shape".to_string(),
                    value: HkxValue::Pointer(Some(1)),
                }])]),
            }],
        };
        HkxFile::from_tagxml(11, "hk_2018.1.0-r1", vec![physics_system, shape])
    }

    fn primitive_shape(class_name: &str, mut members: Vec<HkxMember>) -> HkxObject {
        members.push(HkxMember {
            name: "properties".to_string(),
            value: HkxValue::Pointer(None),
        });
        HkxObject {
            name: Some("#0002".to_string()),
            offset: 0,
            signature: 0,
            class_name: class_name.to_string(),
            members,
        }
    }

    #[test]
    fn direct_primitive_extraction_reads_semantic_sphere_capsule_and_convex_fields() {
        let sphere = direct_body_file(primitive_shape(
            "hknpSphereShape",
            vec![
                HkxMember {
                    name: "convexRadius".to_string(),
                    value: HkxValue::F32(0.25),
                },
                HkxMember {
                    name: "vertices".to_string(),
                    value: HkxValue::Array(vec![HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.5])]),
                },
            ],
        ));
        assert_eq!(
            direct_source_primitive_for_body(&sphere, 0),
            Some(SourcePrimitiveShape::Sphere {
                center: [1.0, 2.0, 3.0],
                radius: 0.25,
            })
        );

        let vertices = vec![
            [-0.001, -1.0, -0.001],
            [0.001, -1.0, -0.001],
            [0.001, 1.0, -0.001],
            [-0.001, 1.0, -0.001],
            [-0.001, -1.0, 0.001],
            [0.001, -1.0, 0.001],
            [0.001, 1.0, 0.001],
            [-0.001, 1.0, 0.001],
        ];
        let vertex_values = vertices
            .iter()
            .enumerate()
            .map(|(index, vertex)| {
                HkxValue::F32List(vec![
                    vertex[0],
                    vertex[1],
                    vertex[2],
                    f32::from_bits(0x3F00_0000 + index as u32),
                ])
            })
            .collect();
        let planes = vec![
            [-1.0, 0.0, 0.0, -0.001],
            [1.0, 0.0, 0.0, -0.001],
            [0.0, -1.0, 0.0, -1.0],
            [0.0, 1.0, 0.0, -1.0],
            [0.0, 0.0, -1.0, -0.001],
            [0.0, 0.0, 1.0, -0.001],
        ]
        .into_iter()
        .map(|plane| HkxValue::F32List(plane.to_vec()))
        .collect();
        let faces = (0..6)
            .map(|index| {
                HkxValue::Object(vec![
                    HkxMember {
                        name: "firstIndex".to_string(),
                        value: HkxValue::U16(index * 4),
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
        let capsule = direct_body_file(primitive_shape(
            "hknpCapsuleShape",
            vec![
                HkxMember {
                    name: "convexRadius".to_string(),
                    value: HkxValue::F32(0.099),
                },
                HkxMember {
                    name: "vertices".to_string(),
                    value: HkxValue::Array(vertex_values),
                },
                HkxMember {
                    name: "planes".to_string(),
                    value: HkxValue::Array(planes),
                },
                HkxMember {
                    name: "faces".to_string(),
                    value: HkxValue::Array(faces),
                },
                HkxMember {
                    name: "indices".to_string(),
                    value: HkxValue::Array((0..24).map(|index| HkxValue::U8(index % 8)).collect()),
                },
                HkxMember {
                    name: "a".to_string(),
                    value: HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.1]),
                },
                HkxMember {
                    name: "b".to_string(),
                    value: HkxValue::F32List(vec![0.0, -1.0, 0.0, 1.0]),
                },
            ],
        ));
        let Some(SourcePrimitiveShape::Capsule(capsule)) =
            direct_source_primitive_for_body(&capsule, 0)
        else {
            panic!("capsule semantic extraction failed");
        };
        assert_eq!(capsule.hull.vertices, vertices);
        assert_eq!(capsule.a[3], 0.1);
        assert_eq!(capsule.convex_radius, 0.099);

        let convex = direct_body_file(primitive_shape(
            "hknpConvexShape",
            vec![
                HkxMember {
                    name: "convexRadius".to_string(),
                    value: HkxValue::F32(0.0),
                },
                HkxMember {
                    name: "vertices".to_string(),
                    value: HkxValue::Array(vec![
                        HkxValue::F32List(vec![-1.0, 0.0, 0.0, 0.5]),
                        HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.5]),
                    ]),
                },
            ],
        ));
        let Some(SourcePrimitiveShape::Convex(convex)) =
            direct_source_primitive_for_body(&convex, 0)
        else {
            panic!("convex semantic extraction failed");
        };
        assert_eq!(convex.vertices.len(), 2);
    }

    #[test]
    fn source_box_extraction_uses_inherited_polytope_topology() {
        let box_shape = primitive_shape(
            "hknpBoxShape",
            vec![
                HkxMember {
                    name: "convexRadius".to_string(),
                    value: HkxValue::F32(0.05),
                },
                HkxMember {
                    name: "vertices".to_string(),
                    value: HkxValue::Array(vec![
                        HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.5]),
                        HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.5]),
                        HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.5]),
                        HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.5]),
                    ]),
                },
                HkxMember {
                    name: "planes".to_string(),
                    value: HkxValue::Array(vec![
                        HkxValue::F32List(vec![-1.0, 0.0, 0.0, 0.0]),
                        HkxValue::F32List(vec![0.0, -1.0, 0.0, 0.0]),
                        HkxValue::F32List(vec![0.0, 0.0, -1.0, 0.0]),
                        HkxValue::F32List(vec![0.577, 0.577, 0.577, -0.577]),
                    ]),
                },
                HkxMember {
                    name: "faces".to_string(),
                    value: HkxValue::Array(
                        (0..4)
                            .map(|index| {
                                HkxValue::Object(vec![
                                    HkxMember {
                                        name: "firstIndex".to_string(),
                                        value: HkxValue::U16(index * 3),
                                    },
                                    HkxMember {
                                        name: "numIndices".to_string(),
                                        value: HkxValue::U8(3),
                                    },
                                    HkxMember {
                                        name: "minHalfAngle".to_string(),
                                        value: HkxValue::U8(4),
                                    },
                                ])
                            })
                            .collect(),
                    ),
                },
                HkxMember {
                    name: "indices".to_string(),
                    value: HkxValue::Array(
                        [0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3]
                            .into_iter()
                            .map(HkxValue::U8)
                            .collect(),
                    ),
                },
            ],
        );
        let file = direct_body_file(box_shape);
        let shapes = extract_source_polytopes_from_hkx(&file, 0);
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].vertices.len(), 4);
        assert_eq!(shapes[0].faces.len(), 4);
        assert_eq!(shapes[0].indices.len(), 12);
        assert_eq!(shapes[0].convex_radius, 0.05);
    }

    #[test]
    fn source_polytope_transform_scales_vertices_planes_and_radius() {
        let shape = SourcePolytopeShape {
            vertices: vec![
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            planes: vec![
                [-1.0, 0.0, 0.0, -0.5],
                [1.0, 0.0, 0.0, -0.5],
                [0.0, -1.0, 0.0, -0.5],
                [0.0, 1.0, 0.0, -0.5],
                [0.0, 0.0, -1.0, -0.5],
                [0.0, 0.0, 1.0, -0.5],
            ],
            faces: vec![
                (0, 4, 128),
                (4, 4, 128),
                (8, 4, 128),
                (12, 4, 128),
                (16, 4, 128),
                (20, 4, 128),
            ],
            indices: vec![
                0, 4, 7, 3, 1, 2, 6, 5, 0, 1, 5, 4, 3, 7, 6, 2, 0, 3, 2, 1, 4, 5, 6, 7,
            ],
            convex_radius: 0.02,
            mass_properties: None,
        };
        let transform = ShapeTransform {
            basis: [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]],
            translation: [1.0, -2.0, 0.5],
        };

        let transformed = transform_source_polytope(shape, transform).expect("transformed shape");

        assert_eq!(transformed.vertices[0], [0.0, -3.5, -1.5]);
        assert!(
            transformed
                .planes
                .iter()
                .any(|plane| approx_plane(*plane, [1.0, 0.0, 0.0, -2.0])),
            "positive x plane should move to x=2.0: {:?}",
            transformed.planes
        );
        assert!(
            (transformed.convex_radius - 0.08).abs() < 1e-6,
            "convex radius should scale by max axis"
        );
    }

    fn approx_plane(actual: [f32; 4], expected: [f32; 4]) -> bool {
        actual
            .iter()
            .zip(expected.iter())
            .all(|(a, e)| (a - e).abs() < 1e-6)
    }

    #[test]
    fn triangles_from_faces_skips_spans_beyond_indices() {
        let faces = [(25, 3, 0), (0, 3, 0)];
        let indices = [0, 1, 2];

        assert_eq!(
            triangles_from_faces_usize(&faces, &indices),
            vec![[0, 1, 2]]
        );
    }

    #[test]
    fn compressed_mesh_preview_ignores_unused_shared_index_metadata() {
        let raw = RawCompressedMeshData {
            user_data: 0,
            edge_welding_map: RawCompressedMeshSparseMap::default(),
            quad_is_flat: RawCompressedMeshBitField::default(),
            triangle_is_interior: RawCompressedMeshBitField::default(),
            materials: Vec::new(),
            object_aabb_min: [0.0, 0.0, 0.0],
            object_aabb_max: [1.0, 1.0, 1.0],
            num_primitive_keys: 1,
            bits_per_key: 8,
            max_key_value: 1,
            primitive_stores_is_flat_convex: 0xff,
            master_tree_nodes: Vec::new(),
            sections: vec![RawCompressedMeshSection {
                aabb_min: [0.0, 0.0, 0.0],
                aabb_max: [1.0, 1.0, 1.0],
                base: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                packed_vertices: vec![pack_vertex_11_11_10(0, 0, 0), pack_vertex_11_11_10(1, 0, 0)],
                shared_vertices_index: vec![9999, 0],
                primitive_bytes: vec![2, 2, 2, 2, 0, 1, 3, 3],
                section_tree_nodes: Vec::new(),
                primitive_data_runs: Vec::new(),
                leaf_index: 0,
                page: 0,
                flags: 1,
                layer_data: 0,
                unused_data: 0,
            }],
            shared_vertices: vec![pack_vertex_21_21_22(0, 1, 0)],
        };

        let mesh = preview_compressed_mesh_from_raw(&raw, 1.0).expect("preview mesh");

        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
    }

    #[test]
    fn compressed_mesh_preview_decodes_custom_flat_convex_local4() {
        // CUSTOM flat-convex primitive (the SCOL CM*.NIF shared-vertex case):
        // m_indices = [svi_record, aabb_node, CUSTOM(3), aabb_node] with the
        // record header / firstShared packed in sharedVerticesIndex and the
        // vertices LOCAL_4-compressed in the u64 shared pool (read as u32 with
        // fetchIndex pair-swapping). Decoded corners form a unit tetrahedron:
        //   V0=(0,0,0) V1=(1,0,0) V2=(0,1,0) V3=(0,0,1)
        // The fetchIndex pair-swap means pool u32 order is [V1,V0,V3,V2].
        let w0 = 0x7FFu32; // x = 2047  -> (1,0,0)  (read for j=1)
        let w1 = 0u32; // (0,0,0)         (read for j=0)
        let w2 = 0xFFC00000u32; // z = 1023 -> (0,0,1) (read for j=3)
        let w3 = 0x003FF800u32; // y = 2047 -> (0,1,0) (read for j=2)
        let pool = vec![
            (w0 as u64) | ((w1 as u64) << 32),
            (w2 as u64) | ((w3 as u64) << 32),
        ];
        // header: numVertices=4 (<<8), compression=LOCAL_4(1) (<<4), type=CUSTOM(3)
        let header = (4u16 << 8) | (1u16 << 4) | 3u16;

        let raw = RawCompressedMeshData {
            user_data: 0,
            edge_welding_map: RawCompressedMeshSparseMap::default(),
            quad_is_flat: RawCompressedMeshBitField::default(),
            triangle_is_interior: RawCompressedMeshBitField::default(),
            materials: Vec::new(),
            object_aabb_min: [0.0, 0.0, 0.0],
            object_aabb_max: [1.0, 1.0, 1.0],
            num_primitive_keys: 1,
            bits_per_key: 8,
            max_key_value: 1,
            primitive_stores_is_flat_convex: 0xff,
            master_tree_nodes: Vec::new(),
            sections: vec![RawCompressedMeshSection {
                aabb_min: [0.0, 0.0, 0.0],
                aabb_max: [1.0, 1.0, 1.0],
                base: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                packed_vertices: Vec::new(),
                shared_vertices_index: vec![header, 0],
                // FO76 custom layout: m_indices = [sviRecord=0, aabbNode, aabbNode,
                // aabbNode]; node 0 == root => AABB is the section domain.
                primitive_bytes: vec![0, 0, 0, 0],
                // single root node so getNodeAabb(0) returns the section domain
                section_tree_nodes: vec![0, 0, 0, 0],
                primitive_data_runs: Vec::new(),
                leaf_index: 0,
                page: 0,
                flags: 1,
                layer_data: 0,
                unused_data: 0,
            }],
            shared_vertices: pool,
        };

        let mesh = preview_compressed_mesh_from_raw(&raw, 1.0).expect("preview mesh");

        assert_eq!(mesh.vertices.len(), 4, "tetra hull keeps all 4 corners");
        assert_eq!(mesh.triangles.len(), 4, "tetra hull has 4 triangular faces");
        for expected in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ] {
            assert!(
                mesh.vertices.iter().any(|v| {
                    (v[0] - expected[0]).abs() < 1e-4
                        && (v[1] - expected[1]).abs() < 1e-4
                        && (v[2] - expected[2]).abs() < 1e-4
                }),
                "decoded hull missing corner {expected:?}; got {:?}",
                mesh.vertices
            );
        }
    }

    #[test]
    fn compressed_mesh_preview_decodes_custom_capsule_local4() {
        let endpoint_b = 0x7FFu32;
        let endpoint_a = 0u32;
        let pool = vec![(endpoint_b as u64) | ((endpoint_a as u64) << 32)];
        // header: numVertices=2, numTags=1, compression=LOCAL_4(1), type=CAPSULE(1)
        let header = (2u16 << 8) | (1u16 << 6) | (1u16 << 4) | 1u16;

        let raw = RawCompressedMeshData {
            user_data: 0,
            edge_welding_map: RawCompressedMeshSparseMap::default(),
            quad_is_flat: RawCompressedMeshBitField::default(),
            triangle_is_interior: RawCompressedMeshBitField::default(),
            materials: Vec::new(),
            object_aabb_min: [0.0, 0.0, 0.0],
            object_aabb_max: [1.0, 1.0, 1.0],
            num_primitive_keys: 1,
            bits_per_key: 8,
            max_key_value: 1,
            primitive_stores_is_flat_convex: 0xff,
            master_tree_nodes: Vec::new(),
            sections: vec![RawCompressedMeshSection {
                aabb_min: [0.0, 0.0, 0.0],
                aabb_max: [1.0, 1.0, 1.0],
                base: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                packed_vertices: Vec::new(),
                shared_vertices_index: vec![header, 0, 0x3E80],
                primitive_bytes: vec![0, 0, 0, 0],
                section_tree_nodes: vec![0, 0, 0, 0],
                primitive_data_runs: Vec::new(),
                leaf_index: 0,
                page: 0,
                flags: 1,
                layer_data: 0,
                unused_data: 0,
            }],
            shared_vertices: pool,
        };

        let mesh = preview_compressed_mesh_from_raw(&raw, 1.0).expect("preview mesh");

        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.triangles.is_empty());
        let min_x = mesh
            .vertices
            .iter()
            .map(|v| v[0])
            .fold(f32::INFINITY, f32::min);
        let max_x = mesh
            .vertices
            .iter()
            .map(|v| v[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            min_x < -0.2 && max_x > 1.2,
            "capsule radius tag should expand the segment, got min_x={min_x} max_x={max_x}"
        );
    }

    #[test]
    fn compressed_mesh_preview_triangulates_custom_flat_convex_local4() {
        let v0 = 0u32;
        let v1 = 0x7FFu32;
        let v2 = 0x003F_FFFFu32;
        let v3 = 0x003F_F800u32;
        let pool = vec![
            (v1 as u64) | ((v0 as u64) << 32),
            (v3 as u64) | ((v2 as u64) << 32),
        ];
        // header: numVertices=4, numTags=0, compression=LOCAL_4(1), type=CONVEX(2)
        let header = (4u16 << 8) | (1u16 << 4) | 2u16;

        let raw = RawCompressedMeshData {
            user_data: 0,
            edge_welding_map: RawCompressedMeshSparseMap::default(),
            quad_is_flat: RawCompressedMeshBitField::default(),
            triangle_is_interior: RawCompressedMeshBitField::default(),
            materials: Vec::new(),
            object_aabb_min: [0.0, 0.0, 0.0],
            object_aabb_max: [1.0, 1.0, 1.0],
            num_primitive_keys: 1,
            bits_per_key: 8,
            max_key_value: 1,
            primitive_stores_is_flat_convex: 0xff,
            master_tree_nodes: Vec::new(),
            sections: vec![RawCompressedMeshSection {
                aabb_min: [0.0, 0.0, 0.0],
                aabb_max: [1.0, 1.0, 1.0],
                base: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                packed_vertices: Vec::new(),
                shared_vertices_index: vec![header, 0],
                primitive_bytes: vec![0, 0, 0, 0],
                section_tree_nodes: vec![0, 0, 0, 0],
                primitive_data_runs: Vec::new(),
                leaf_index: 0,
                page: 0,
                flags: 1,
                layer_data: 0,
                unused_data: 0,
            }],
            shared_vertices: pool,
        };

        let mesh = preview_compressed_mesh_from_raw(&raw, 1.0).expect("preview mesh");

        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.triangles.len(), 2);
        assert!(
            mesh.vertices.iter().all(|vertex| vertex[2].abs() < 1.0e-5),
            "flat convex preview should stay on the source plane: {:?}",
            mesh.vertices
        );
    }

    #[test]
    fn aabb_alignment_basis_preserves_up_for_xy_swaps() {
        let local = super::ShapeAabb {
            min: [-10.0, -2.0, -1.0],
            max: [10.0, 2.0, 1.0],
        };
        let target = super::ShapeAabb {
            min: [-2.0, -10.0, -1.0],
            max: [2.0, 10.0, 1.0],
        };

        let basis = super::aabb_alignment_basis(local, target);

        assert!(
            basis[2][2] > 0.9,
            "AABB alignment must preserve local +Z; got {basis:?}"
        );
        assert!(
            basis[0][1].abs() > 0.9 && basis[1][0].abs() > 0.9,
            "AABB alignment should still rotate the horizontal axes; got {basis:?}"
        );
        assert!(
            super::determinant3(basis) > 0.0,
            "AABB alignment must remain a proper basis; got {basis:?}"
        );
    }

    #[test]
    fn ordered_leaf_aabbs_from_pairs_uses_encoded_leaf_order() {
        let first = super::ShapeAabb {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };
        let second = super::ShapeAabb {
            min: [10.0, 0.0, 0.0],
            max: [11.0, 1.0, 1.0],
        };
        let third = super::ShapeAabb {
            min: [20.0, 0.0, 0.0],
            max: [21.0, 1.0, 1.0],
        };

        let ordered =
            super::ordered_leaf_aabbs_from_pairs(&[(2, third), (0, first), (1, second)], 3)
                .expect("ordered leaf aabbs");

        assert_eq!(super::aabb_center(ordered[0]), [0.5, 0.5, 0.5]);
        assert_eq!(super::aabb_center(ordered[1]), [10.5, 0.5, 0.5]);
        assert_eq!(super::aabb_center(ordered[2]), [20.5, 0.5, 0.5]);
        assert!(
            super::ordered_leaf_aabbs_from_pairs(&[(0, first), (0, second)], 2).is_none(),
            "duplicate or incomplete leaf maps must fall back to unordered matching"
        );
    }

    #[test]
    fn shared_pool_index_applies_page_offset() {
        assert_eq!(shared_pool_index(0, 5), 5);
        assert_eq!(shared_pool_index(1, 0), 65536);
        assert_eq!(shared_pool_index(2, 7), 131079);
    }

    #[test]
    fn shape_instance_transform_reads_column_major() {
        // Column-major hkTransform: col0=[0..3], col1=[4..7], col2=[8..11], translation=[12..15].
        // 90-deg rotation about Z, translation (5,6,7):
        //   col0=(0,1,0,*), col1=(-1,0,0,*), col2=(0,0,1,*), trans=(5,6,7,*)
        use super::shape_instance_transform;
        use crate::hkx::model::HkxMember;
        use crate::hkx::types::HkxValue;
        let values = vec![
            0.0f32, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0,
        ];
        let members = vec![HkxMember {
            name: "transform".to_string(),
            value: HkxValue::F32List(values),
        }];
        let t = shape_instance_transform(&members);
        assert_eq!(t.translation, [5.0, 6.0, 7.0]);
        assert_eq!(t.basis[0], [0.0, -1.0, 0.0]);
        assert_eq!(t.basis[1], [1.0, 0.0, 0.0]);
        assert_eq!(t.basis[2], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn compound_leaf_inference_excludes_cross_body_shapes() {
        // FO76 norm: a body's compound shape whose `instances` array does not
        // decode (empty) and whose backing `hknpDynamicCompoundShapeData` is
        // present in the blob but NOT reachable from this compound (no resolved
        // `boundingVolumeData` pointer). A SECOND, unrelated leaf shape belongs
        // to a different body. Compound-leaf inference must never claim that
        // cross-body leaf as one of this compound's children — the SDK only
        // enumerates membership through the compound's own backing data, never
        // "every previewable leaf in the file".
        use super::inferred_compound_leaf_targets;
        use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
        use crate::hkx::types::HkxValue;

        fn object(name: &str, class: &str, members: Vec<HkxMember>) -> HkxObject {
            HkxObject {
                name: Some(name.to_string()),
                offset: 0,
                signature: 0,
                class_name: class.to_string(),
                members,
            }
        }
        fn ptr(name: &str, target: usize) -> HkxMember {
            HkxMember {
                name: name.to_string(),
                value: HkxValue::Pointer(Some(target)),
            }
        }

        let aabb_tree = HkxValue::Object(vec![HkxMember {
            name: "numLeaves".to_string(),
            value: HkxValue::U32(1),
        }]);
        let body1_instances = HkxValue::Array(vec![HkxValue::Object(vec![ptr("shape", 4)])]);
        let objects = vec![
            object(
                "#0000",
                "hknpStaticCompoundShape",
                vec![HkxMember {
                    name: "instances".to_string(),
                    value: HkxValue::Array(Vec::new()),
                }],
            ),
            object(
                "#0001",
                "hknpDynamicCompoundShapeData",
                vec![HkxMember {
                    name: "aabbTree".to_string(),
                    value: aabb_tree,
                }],
            ),
            object("#0002", "hknpConvexPolytopeShape", Vec::new()),
            object(
                "#0003",
                "hknpStaticCompoundShape",
                vec![HkxMember {
                    name: "instances".to_string(),
                    value: body1_instances,
                }],
            ),
            object("#0004", "hknpConvexPolytopeShape", Vec::new()),
        ];
        let hkx = HkxFile::from_tagxml(0, "test", objects);

        let compound_index = 0usize;
        // Body 0's shape is compound 0; body 1's shape is compound 3. Only the
        // compounds are body shapes — their nested leaves (#0002, #0004) are not.
        let body_shape_indices = [compound_index, 3usize];
        let compound = &hkx.objects()[compound_index];

        let targets =
            inferred_compound_leaf_targets(&hkx, compound_index, compound, &body_shape_indices);

        assert!(
            !targets.iter().any(|target| target.index == 4),
            "cross-body leaf #0004 (body 1's) must never be claimed as a child of compound #0000; got {targets:?}"
        );
        // The compound's own backing data is unreachable here, so there is no
        // authoritative leaf set — inference must yield nothing rather than
        // sweeping the whole blob.
        assert!(
            targets.is_empty(),
            "with no reachable backing data, inference must return no leaves; got {targets:?}"
        );
    }
}

fn fan_triangles(n: usize) -> Vec<[u32; 3]> {
    if n < 3 {
        return Vec::new();
    }
    (1..n as u32 - 1).map(|i| [0, i, i + 1]).collect()
}

fn average_xyz(verts: &[[f32; 3]]) -> [f32; 3] {
    if verts.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    let n = verts.len() as f32;
    let sx: f32 = verts.iter().map(|v| v[0]).sum();
    let sy: f32 = verts.iter().map(|v| v[1]).sum();
    let sz: f32 = verts.iter().map(|v| v[2]).sum();
    [sx / n, sy / n, sz / n]
}

fn normalize_vector(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= 1e-8 {
        return [1.0, 0.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn orthonormal_basis(direction: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let axis = normalize_vector(direction);
    let reference = if axis[2].abs() < 0.95 {
        [0.0_f32, 0.0, 1.0]
    } else {
        [0.0_f32, 1.0, 0.0]
    };
    let u = normalize_vector(cross(reference, axis));
    let v = normalize_vector(cross(axis, u));
    (u, v)
}

// ---------------------------------------------------------------------------
// Member accessors
// ---------------------------------------------------------------------------

fn member_f32(obj: &HkxObject, name: &str) -> f32 {
    for m in &obj.members {
        if m.name == name {
            return match m.value {
                HkxValue::F32(v) => v,
                HkxValue::F32List(ref list) if !list.is_empty() => list[0],
                _ => 0.0,
            };
        }
    }
    0.0
}

fn member_vec3(obj: &HkxObject, name: &str) -> Option<[f32; 3]> {
    for m in &obj.members {
        if m.name == name {
            return value_vec3(&m.value);
        }
    }
    None
}

fn member_vec3_from_members(members: &[HkxMember], name: &str) -> Option<[f32; 3]> {
    members.iter().find_map(|member| {
        if member.name == name {
            value_vec3(&member.value)
        } else {
            None
        }
    })
}

fn member_vec4_from_members(members: &[HkxMember], name: &str) -> Option<[f32; 4]> {
    members.iter().find_map(|member| {
        if member.name == name {
            value_vec4(&member.value)
        } else {
            None
        }
    })
}

fn member_vec4_array(obj: &HkxObject, name: &str) -> Vec<[f32; 3]> {
    for m in &obj.members {
        if m.name == name {
            if let HkxValue::Array(ref items) = m.value {
                return items
                    .iter()
                    .filter_map(|item| match item {
                        HkxValue::F32List(list) if list.len() >= 3 => {
                            Some([list[0], list[1], list[2]])
                        }
                        _ => None,
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

fn member_vec4_array4(obj: &HkxObject, name: &str) -> Vec<[f32; 4]> {
    for m in &obj.members {
        if m.name == name {
            if let HkxValue::Array(ref items) = m.value {
                return items.iter().filter_map(|item| value_vec4(item)).collect();
            }
        }
    }
    Vec::new()
}

fn member_pointer(obj: &HkxObject, name: &str) -> Option<usize> {
    obj.members.iter().find_map(|member| {
        if member.name == name {
            match &member.value {
                HkxValue::Pointer(index) => *index,
                _ => None,
            }
        } else {
            None
        }
    })
}

fn member_target_index(hkx: &HkxFile, members: &[HkxMember], name: &str) -> Option<usize> {
    members.iter().find_map(|member| {
        if member.name != name {
            return None;
        }
        match &member.value {
            HkxValue::Pointer(index) => *index,
            HkxValue::String { value, .. } if !value.is_empty() => hkx
                .objects()
                .iter()
                .position(|object| object.name.as_deref() == Some(value.as_str())),
            _ => None,
        }
    })
}

fn member_object<'a>(obj: &'a HkxObject, name: &str) -> Option<&'a [HkxMember]> {
    obj.members.iter().find_map(|member| {
        if member.name == name {
            member.value.as_object_members()
        } else {
            None
        }
    })
}

fn member_object_from_members<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a [HkxMember]> {
    members.iter().find_map(|member| {
        if member.name == name {
            member.value.as_object_members()
        } else {
            None
        }
    })
}

fn value_object(value: &HkxValue) -> Option<&[HkxMember]> {
    value.as_object_members()
}

fn member_array<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a [HkxValue]> {
    members.iter().find_map(|member| {
        if member.name == name {
            match &member.value {
                HkxValue::Array(values) => Some(values.as_slice()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn member_u32(members: &[HkxMember], name: &str) -> Option<u32> {
    members.iter().find_map(|member| {
        if member.name == name {
            hkx_value_as_u32(&member.value)
        } else {
            None
        }
    })
}

fn nested_member_u32(members: &[HkxMember], object_name: &str, value_name: &str) -> Option<u32> {
    let nested = members.iter().find_map(|member| {
        if member.name == object_name {
            member.value.as_object_members()
        } else {
            None
        }
    })?;
    member_u32(nested, value_name)
}

fn value_u32(value: &HkxValue) -> Option<u32> {
    hkx_value_as_u32(value)
}

fn value_u64(value: &HkxValue) -> Option<u64> {
    match value {
        HkxValue::U64(v) => Some(*v),
        HkxValue::I64(v) => u64::try_from(*v).ok(),
        HkxValue::U32(v) => Some(*v as u64),
        HkxValue::I32(v) => u64::try_from(*v).ok(),
        _ => None,
    }
}

fn value_bits_u64(value: &HkxValue) -> Option<u64> {
    match value {
        HkxValue::U64(v) => Some(*v),
        HkxValue::I64(v) => Some(*v as u64),
        HkxValue::U32(v) => Some(*v as u64),
        HkxValue::I32(v) => Some(*v as u32 as u64),
        _ => None,
    }
}

fn value_f32(value: &HkxValue) -> Option<f32> {
    match value {
        HkxValue::F32(v) => Some(*v),
        HkxValue::Half(v) => Some(*v),
        HkxValue::F32List(list) if !list.is_empty() => Some(list[0]),
        HkxValue::I32(v) => Some(*v as f32),
        HkxValue::U32(v) => Some(*v as f32),
        _ => None,
    }
}

fn value_vec3(value: &HkxValue) -> Option<[f32; 3]> {
    match value {
        HkxValue::F32List(list) if list.len() >= 3 => Some([list[0], list[1], list[2]]),
        HkxValue::Array(values) if values.len() >= 3 => Some([
            value_f32(&values[0])?,
            value_f32(&values[1])?,
            value_f32(&values[2])?,
        ]),
        _ => None,
    }
}

fn value_vec4(value: &HkxValue) -> Option<[f32; 4]> {
    match value {
        HkxValue::F32List(list) if list.len() >= 4 => Some([list[0], list[1], list[2], list[3]]),
        HkxValue::Array(values) if values.len() >= 4 => Some([
            value_f32(&values[0])?,
            value_f32(&values[1])?,
            value_f32(&values[2])?,
            value_f32(&values[3])?,
        ]),
        _ => None,
    }
}

fn uint8_array_member(obj: &HkxObject, name: &str) -> Vec<u8> {
    for m in &obj.members {
        if m.name == name {
            if let HkxValue::Array(ref items) = m.value {
                return items
                    .iter()
                    .filter_map(|item| match item {
                        HkxValue::U8(v) => Some(*v),
                        HkxValue::I32(v) => Some(*v as u8),
                        HkxValue::U32(v) => Some(*v as u8),
                        _ => None,
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

fn face_tuples_member(obj: &HkxObject, name: &str) -> Vec<(u32, u32, u32)> {
    for m in &obj.members {
        if m.name == name {
            if let HkxValue::Array(ref items) = m.value {
                return items
                    .iter()
                    .filter_map(|item| {
                        let members = item.as_object_members()?;
                        let first = find_u32_in_members(members, "firstIndex");
                        let num = find_u32_in_members(members, "numIndices");
                        let angle = find_u32_in_members(members, "minHalfAngle");
                        Some((first, num, angle))
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

fn find_u32_in_members(members: &[HkxMember], name: &str) -> u32 {
    for m in members {
        if m.name == name {
            return hkx_value_as_u32(&m.value).unwrap_or(0);
        }
    }
    0
}

fn hkx_value_as_u32(value: &HkxValue) -> Option<u32> {
    match value {
        HkxValue::Bool(v) => Some(u32::from(*v)),
        HkxValue::I8(v) => Some((*v).max(0) as u32),
        HkxValue::U8(v) => Some(*v as u32),
        HkxValue::I16(v) => Some((*v).max(0) as u32),
        HkxValue::U16(v) => Some(*v as u32),
        HkxValue::I32(v) => Some((*v).max(0) as u32),
        HkxValue::U32(v) => Some(*v),
        HkxValue::I64(v) => Some((*v).max(0) as u32),
        HkxValue::U64(v) => Some((*v).min(u32::MAX as u64) as u32),
        HkxValue::F32(v) => Some((*v).max(0.0) as u32),
        HkxValue::Half(v) => Some((*v).max(0.0) as u32),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Body scoping helper
// ---------------------------------------------------------------------------

/// Resolve the object indices of the shape pointer for `bodyCinfos[body_id]`.
///
/// Pointers may surface as either `HkxValue::Pointer(Some(target_index))` (binary
/// packfile path, where the reader resolves global fixups to object indices) or
/// `HkxValue::String { value: "#NNNN" }` (tagxml path, where pointers are
/// stored by synthesized object name). Both forms are mapped back to object
/// indices so the caller can filter `hkx.objects()` by position.
fn shape_targets_for_body(hkx: &HkxFile, body_id: Option<usize>) -> Option<Vec<ShapeTarget>> {
    let body_id = body_id?;
    let body_shape_indices = body_shape_indices(hkx);
    let shape_index = body_shape_index_for_body(hkx, body_id)?;
    Some(shape_targets_for_shape_index(
        hkx,
        shape_index,
        ShapeTransform::identity(),
        &body_shape_indices,
    ))
}

fn body_shape_index_for_body(hkx: &HkxFile, body_id: usize) -> Option<usize> {
    let psd = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hknpPhysicsSystemData")?;
    for m in &psd.members {
        if m.name != "bodyCinfos" {
            continue;
        }
        if let HkxValue::Array(ref bodies) = m.value {
            let body_val = bodies.get(body_id)?;
            if let Some(body_members) = body_val.as_object_members() {
                if let Some(idx) = member_target_index(hkx, body_members, "shape") {
                    return Some(idx);
                }
            }
        }
        return None;
    }
    None
}

fn shape_targets_for_shape_index(
    hkx: &HkxFile,
    shape_index: usize,
    transform: ShapeTransform,
    body_shape_indices: &[usize],
) -> Vec<ShapeTarget> {
    if let Some(obj) = hkx.objects().get(shape_index) {
        if is_hknp_compound_shape(&obj.class_name) {
            return compound_child_shape_targets(
                hkx,
                shape_index,
                obj,
                transform,
                body_shape_indices,
            );
        }
    }
    vec![ShapeTarget {
        index: shape_index,
        transform,
    }]
}

fn body_shape_class_for_body(hkx: &HkxFile, body_id: Option<usize>) -> Option<String> {
    let body_id = body_id?;
    let target_index = body_shape_index_for_body(hkx, body_id)?;
    hkx.objects()
        .get(target_index)
        .map(|object| object.class_name.clone())
}

fn compound_child_shape_targets(
    hkx: &HkxFile,
    compound_index: usize,
    obj: &HkxObject,
    parent_transform: ShapeTransform,
    body_shape_indices: &[usize],
) -> Vec<ShapeTarget> {
    let mut targets = Vec::new();

    if let Some(elements) = compound_instance_elements(obj) {
        for element in elements {
            let Some(members) = element.as_object_members() else {
                continue;
            };
            if let Some(index) = member_target_index(hkx, members, "shape") {
                let transform = parent_transform.compose(shape_instance_transform(members));
                targets.extend(shape_targets_for_shape_index(
                    hkx,
                    index,
                    transform,
                    body_shape_indices,
                ));
            }
        }
        if !targets.is_empty() {
            return targets;
        }
    }

    inferred_compound_leaf_targets(hkx, compound_index, obj, body_shape_indices)
        .into_iter()
        .map(|target| ShapeTarget {
            index: target.index,
            transform: parent_transform.compose(target.transform),
        })
        .collect()
}

fn compound_instance_elements(obj: &HkxObject) -> Option<&[HkxValue]> {
    let member = obj
        .members
        .iter()
        .find(|member| member.name == "instances")?;
    match &member.value {
        HkxValue::Array(values) => Some(values.as_slice()),
        _ => {
            let members = member.value.as_object_members()?;
            member_array(members, "elements")
        }
    }
}

fn inferred_compound_leaf_targets(
    hkx: &HkxFile,
    _compound_index: usize,
    obj: &HkxObject,
    body_shape_indices: &[usize],
) -> Vec<ShapeTarget> {
    let wrapped_shapes = wrapped_shape_indices(hkx);
    // The compound's own backing data (`hknpDynamicCompoundShapeData` /
    // `numLeaves`) is the only authoritative membership source. When it
    // resolves, use it directly. When it does not, return nothing: the
    // previous blanket whole-blob sweep treated EVERY previewable leaf in the
    // file as this compound's child, pulling in shapes that belong to other
    // bodies (the "×3 dup" / cross-body bleed) and producing degenerate hulls
    // downstream. The SDK never enumerates "every leaf in the file" as compound
    // membership.
    compound_backing_data_leaf_targets(hkx, obj, body_shape_indices, &wrapped_shapes)
        .unwrap_or_default()
}

fn compound_backing_data_leaf_targets(
    hkx: &HkxFile,
    compound: &HkxObject,
    body_shape_indices: &[usize],
    wrapped_shapes: &[usize],
) -> Option<Vec<ShapeTarget>> {
    let data_index = member_target_index(hkx, &compound.members, "boundingVolumeData")?;
    let data = hkx.objects().get(data_index)?;
    if data.class_name != "hknpDynamicCompoundShapeData" {
        return None;
    }
    let leaf_count = dynamic_compound_leaf_count(data)?;
    if leaf_count == 0 {
        return None;
    }

    let mut targets = Vec::with_capacity(leaf_count);
    for (index, object) in hkx.objects().iter().enumerate().skip(data_index + 1) {
        if object.class_name == "hknpDynamicCompoundShapeData" {
            break;
        }
        if body_shape_indices.contains(&index) || wrapped_shapes.contains(&index) {
            continue;
        }
        if previewable_leaf_shape_class(&object.class_name) {
            targets.push(index);
            if targets.len() == leaf_count {
                let aabbs = dynamic_compound_aabb_candidates(data);
                return Some(shape_targets_from_leaf_aabbs(hkx, &targets, &aabbs));
            }
        }
    }

    None
}

fn shape_targets_from_leaf_aabbs(
    hkx: &HkxFile,
    indices: &[usize],
    aabbs: &ShapeAabbCandidates,
) -> Vec<ShapeTarget> {
    if aabbs.ordered && aabbs.aabbs.len() == indices.len() {
        return indices
            .iter()
            .zip(aabbs.aabbs.iter())
            .map(|(index, target)| ShapeTarget {
                index: *index,
                transform: shape_alignment_transform(hkx, *index, *target),
            })
            .collect();
    }

    let mut unused_aabbs = aabbs.aabbs.clone();
    indices
        .iter()
        .map(|index| {
            let transform = preview_meshes_aabb(&preview_for_shape_index(hkx, *index, 1.0))
                .and_then(|local| {
                    let best = best_matching_aabb_index(&local, &unused_aabbs)?;
                    let target = unused_aabbs.remove(best);
                    Some(aabb_alignment_transform(local, target))
                })
                .unwrap_or_else(ShapeTransform::identity);
            ShapeTarget {
                index: *index,
                transform,
            }
        })
        .collect()
}

fn shape_alignment_transform(hkx: &HkxFile, index: usize, target: ShapeAabb) -> ShapeTransform {
    preview_meshes_aabb(&preview_for_shape_index(hkx, index, 1.0))
        .map(|local| aabb_alignment_transform(local, target))
        .unwrap_or_else(ShapeTransform::identity)
}

fn aabb_alignment_transform(local: ShapeAabb, target: ShapeAabb) -> ShapeTransform {
    let basis = aabb_alignment_basis(local, target);
    let local_center = aabb_center(local);
    let target_center = aabb_center(target);
    let rotated_local_center = transform_basis_point(basis, local_center);
    ShapeTransform {
        basis,
        translation: [
            target_center[0] - rotated_local_center[0],
            target_center[1] - rotated_local_center[1],
            target_center[2] - rotated_local_center[2],
        ],
    }
}

fn aabb_alignment_basis(local: ShapeAabb, target: ShapeAabb) -> [[f32; 3]; 3] {
    let local_extent = aabb_extent(local);
    let target_extent = aabb_extent(target);
    if !local_extent
        .iter()
        .all(|value| value.is_finite() && *value > 1e-5)
        || !target_extent
            .iter()
            .all(|value| value.is_finite() && *value > 1e-5)
    {
        return ShapeTransform::identity().basis;
    }

    let identity = ShapeTransform::identity().basis;
    let identity_score = aabb_alignment_score(identity, local_extent, target_extent);
    let identity_preference = aabb_alignment_preference(identity);
    let mut best_basis = identity;
    let mut best_score = identity_score;
    let mut best_preference = identity_preference;
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let signs = [-1.0, 1.0];
    for permutation in permutations {
        for sx in signs {
            for sy in signs {
                for sz in signs {
                    let mut basis = [[0.0; 3]; 3];
                    basis[0][permutation[0]] = sx;
                    basis[1][permutation[1]] = sy;
                    basis[2][permutation[2]] = sz;
                    if determinant3(basis) <= 0.0 {
                        continue;
                    }
                    let score = aabb_alignment_score(basis, local_extent, target_extent);
                    let preference = aabb_alignment_preference(basis);
                    if score + 1e-5 < best_score
                        || ((score - best_score).abs() <= 1e-5
                            && preference > best_preference + 1e-5)
                    {
                        best_score = score;
                        best_basis = basis;
                        best_preference = preference;
                    }
                }
            }
        }
    }

    if best_score + 1e-3 < identity_score && best_score <= 0.15 {
        best_basis
    } else {
        identity
    }
}

fn aabb_alignment_preference(basis: [[f32; 3]; 3]) -> f32 {
    basis[2][2]
}

fn aabb_alignment_score(
    basis: [[f32; 3]; 3],
    local_extent: [f32; 3],
    target_extent: [f32; 3],
) -> f32 {
    let rotated_extent = [
        basis[0][0].abs() * local_extent[0]
            + basis[0][1].abs() * local_extent[1]
            + basis[0][2].abs() * local_extent[2],
        basis[1][0].abs() * local_extent[0]
            + basis[1][1].abs() * local_extent[1]
            + basis[1][2].abs() * local_extent[2],
        basis[2][0].abs() * local_extent[0]
            + basis[2][1].abs() * local_extent[1]
            + basis[2][2].abs() * local_extent[2],
    ];
    (0..3)
        .map(|axis| {
            let denom = target_extent[axis].abs().max(1.0);
            (rotated_extent[axis] - target_extent[axis]).abs() / denom
        })
        .sum()
}

fn transform_basis_point(basis: [[f32; 3]; 3], point: [f32; 3]) -> [f32; 3] {
    [
        basis[0][0] * point[0] + basis[0][1] * point[1] + basis[0][2] * point[2],
        basis[1][0] * point[0] + basis[1][1] * point[1] + basis[1][2] * point[2],
        basis[2][0] * point[0] + basis[2][1] * point[1] + basis[2][2] * point[2],
    ]
}

fn best_matching_aabb_index(local: &ShapeAabb, candidates: &[ShapeAabb]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_score = aabb_extent_score(*local, **left);
            let right_score = aabb_extent_score(*local, **right);
            left_score
                .partial_cmp(&right_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

fn aabb_extent_score(left: ShapeAabb, right: ShapeAabb) -> f32 {
    let left_extent = aabb_extent(left);
    let right_extent = aabb_extent(right);
    (left_extent[0] - right_extent[0]).abs()
        + (left_extent[1] - right_extent[1]).abs()
        + (left_extent[2] - right_extent[2]).abs()
}

fn dynamic_compound_aabb_candidates(data: &HkxObject) -> ShapeAabbCandidates {
    let Some(tree) = member_object(data, "aabbTree") else {
        return ShapeAabbCandidates::unordered(Vec::new());
    };
    let Some(nodes) = member_array(tree, "nodes") else {
        return ShapeAabbCandidates::unordered(Vec::new());
    };
    let leaf_count = member_u32(tree, "numLeaves").unwrap_or(0);
    let mut all = Vec::new();
    let mut leaves = Vec::new();
    let mut leaf_pairs = Vec::new();
    for node in nodes.iter() {
        if let Some((aabb, leaf_index)) = (|| {
            let node_members = value_object(node)?;
            let aabb = member_object_from_members(node_members, "aabb")?;
            let min4 = member_vec4_from_members(aabb, "min")?;
            let max4 = member_vec4_from_members(aabb, "max")?;
            let shape_aabb = ShapeAabb {
                min: [min4[0], min4[1], min4[2]],
                max: [max4[0], max4[1], max4[2]],
            };
            if !aabb_is_finite(shape_aabb)
                || !aabb_extent(shape_aabb).iter().any(|value| *value > 1e-6)
            {
                return None;
            }
            let max_w_bits = max4[3].to_bits();
            let left_or_zero = max_w_bits & 0xffff;
            let leaf_index = max_w_bits >> 16;
            Some((shape_aabb, (left_or_zero == 0).then_some(leaf_index)))
        })() {
            all.push(aabb);
            if let Some(leaf_index) = leaf_index.filter(|index| *index < leaf_count) {
                leaves.push(aabb);
                leaf_pairs.push((leaf_index, aabb));
            }
        }
    }
    if let Some(ordered) = ordered_leaf_aabbs_from_pairs(&leaf_pairs, leaf_count) {
        return ShapeAabbCandidates::ordered(ordered);
    }
    if leaves.len() >= leaf_count as usize && leaf_count > 0 {
        ShapeAabbCandidates::unordered(leaves)
    } else {
        ShapeAabbCandidates::unordered(all)
    }
}

fn ordered_leaf_aabbs_from_pairs(
    pairs: &[(u32, ShapeAabb)],
    leaf_count: u32,
) -> Option<Vec<ShapeAabb>> {
    let leaf_count = usize::try_from(leaf_count).ok()?;
    if leaf_count == 0 || pairs.len() < leaf_count {
        return None;
    }
    let mut ordered = vec![None; leaf_count];
    for (leaf_index, aabb) in pairs {
        let slot = usize::try_from(*leaf_index).ok()?;
        let item = ordered.get_mut(slot)?;
        if item.is_some() {
            return None;
        }
        *item = Some(*aabb);
    }
    ordered.into_iter().collect()
}

fn preview_meshes_aabb(meshes: &[PreviewMesh]) -> Option<ShapeAabb> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut seen = false;
    for vertex in meshes.iter().flat_map(|mesh| mesh.vertices.iter()) {
        seen = true;
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    seen.then_some(ShapeAabb { min, max })
}

fn aabb_center(aabb: ShapeAabb) -> [f32; 3] {
    [
        (aabb.min[0] + aabb.max[0]) * 0.5,
        (aabb.min[1] + aabb.max[1]) * 0.5,
        (aabb.min[2] + aabb.max[2]) * 0.5,
    ]
}

fn aabb_extent(aabb: ShapeAabb) -> [f32; 3] {
    [
        aabb.max[0] - aabb.min[0],
        aabb.max[1] - aabb.min[1],
        aabb.max[2] - aabb.min[2],
    ]
}

fn aabb_is_finite(aabb: ShapeAabb) -> bool {
    aabb.min
        .iter()
        .chain(aabb.max.iter())
        .all(|value| value.is_finite())
}

fn dynamic_compound_leaf_count(data: &HkxObject) -> Option<usize> {
    let tree = member_object(data, "aabbTree")?;
    let count = member_u32(tree, "numLeaves")?;
    usize::try_from(count).ok()
}

fn wrapped_shape_indices(hkx: &HkxFile) -> Vec<usize> {
    hkx.objects()
        .iter()
        .filter(|object| {
            matches!(
                object.class_name.as_str(),
                "hknpScaledConvexShape" | "hknpScaledConvexShapeBase"
            )
        })
        .filter_map(|object| member_target_index(hkx, &object.members, "coreShape"))
        .collect()
}

fn body_shape_indices(hkx: &HkxFile) -> Vec<usize> {
    let Some(psd) = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
    else {
        return Vec::new();
    };
    let Some(bodies) = psd
        .members
        .iter()
        .find(|member| member.name == "bodyCinfos")
        .and_then(|member| match &member.value {
            HkxValue::Array(items) => Some(items.as_slice()),
            _ => None,
        })
    else {
        return Vec::new();
    };
    bodies
        .iter()
        .filter_map(|body| {
            body.as_object_members()
                .and_then(|members| member_target_index(hkx, members, "shape"))
        })
        .collect()
}

fn previewable_leaf_shape_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "hknpBoxShape"
            | "hkpBoxShape"
            | "hknpCapsuleShape"
            | "hknpCompressedMeshShape"
            | "hknpConvexPolytopeShape"
            | "hkpConvexVerticesShape"
            | "hknpConvexShape"
            | "hknpScaledConvexShape"
            | "hknpScaledConvexShapeBase"
            | "hknpSphereShape"
    )
}

fn shape_instance_transform(members: &[HkxMember]) -> ShapeTransform {
    let Some(transform_member) = members.iter().find(|member| member.name == "transform") else {
        return ShapeTransform::identity();
    };
    let HkxValue::F32List(values) = &transform_member.value else {
        return ShapeTransform::identity();
    };
    if values.len() < 15 {
        return ShapeTransform::identity();
    }

    let rotation_values = [
        values[0], values[1], values[2], values[4], values[5], values[6], values[8], values[9],
        values[10], values[12], values[13], values[14],
    ];
    if rotation_values.iter().any(|value| !value.is_finite()) {
        return ShapeTransform::identity();
    }

    // Havok hkTransform stores rotation as three column vectors followed by
    // translation. Convert that column-major layout into the row-major basis
    // used by ShapeTransform::compose/apply.
    let basis = [
        [values[0], values[4], values[8]],
        [values[1], values[5], values[9]],
        [values[2], values[6], values[10]],
    ];
    let translation = [values[12], values[13], values[14]];

    ShapeTransform { basis, translation }
}

fn is_hknp_compound_shape(class_name: &str) -> bool {
    matches!(
        class_name,
        "hknpCompoundShape" | "hknpDynamicCompoundShape" | "hknpStaticCompoundShape"
    )
}
