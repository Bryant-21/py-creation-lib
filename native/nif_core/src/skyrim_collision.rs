use std::collections::HashSet;

use havok_native::collision::BuildOptions;
use havok_native::collision::compound::{CompoundChild, CompoundChildKind};
use havok_native::collision::multi_body::{
    BodyMeta, BodyMotionType, MultiBodyShape, build_fo4_multi_body_collision,
};

use crate::model::{NifBlock, NifFile, NifValue};

const FO4_STATIC_LAYER: u8 = 1;
const SOURCE_FIXED_MOTION: u64 = 7;
const SOURCE_BOX_STABILIZED_MOTION: u64 = 5;
const DEFAULT_COLLISION_RADIUS: f32 = 0.01;

#[derive(Debug, Default)]
pub(crate) struct SkyrimCollisionReport {
    pub converted: usize,
    pub stripped: usize,
    pub warnings: Vec<String>,
}

struct CollisionPlan {
    parent_id: usize,
    source_collision_id: usize,
    shape: MultiBodyShape,
}

pub(crate) fn bridge_static_collision(nif: &mut NifFile) -> SkyrimCollisionReport {
    let collision_ids = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkCollisionObject")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    if collision_ids.is_empty() {
        return SkyrimCollisionReport::default();
    }

    let mut report = SkyrimCollisionReport::default();
    let mut plans = Vec::new();
    let mut remove = HashSet::new();
    for collision_id in collision_ids {
        let Some(collision) = nif.get_block(collision_id) else {
            continue;
        };
        let parent_id = match ref_field(collision, "Target") {
            Some(id) if nif.get_block(id).is_some() => id,
            _ => {
                report.warnings.push(format!(
                    "Skyrim collision block {collision_id}: missing target; stripped"
                ));
                collect_collision_subtree(nif, collision_id, &mut remove);
                report.stripped += 1;
                continue;
            }
        };
        let body_id = match ref_field(collision, "Body") {
            Some(id) => id,
            None => {
                report.warnings.push(format!(
                    "Skyrim collision block {collision_id}: missing rigid body; stripped"
                ));
                collect_collision_subtree(nif, collision_id, &mut remove);
                report.stripped += 1;
                continue;
            }
        };
        let Some(body) = nif.get_block(body_id) else {
            report.warnings.push(format!(
                "Skyrim collision block {collision_id}: rigid body {body_id} is missing; stripped"
            ));
            collect_collision_subtree(nif, collision_id, &mut remove);
            report.stripped += 1;
            continue;
        };
        if let Err(reason) = validate_static_body(body) {
            report.warnings.push(format!(
                "Skyrim collision block {collision_id}: {reason}; stripped"
            ));
            collect_collision_subtree(nif, collision_id, &mut remove);
            report.stripped += 1;
            continue;
        }
        let Some(shape_id) = ref_field(body, "Shape") else {
            report.warnings.push(format!(
                "Skyrim collision block {collision_id}: rigid body has no shape; stripped"
            ));
            collect_collision_subtree(nif, collision_id, &mut remove);
            report.stripped += 1;
            continue;
        };
        match decode_static_shape(nif, shape_id) {
            Ok(shape) => plans.push(CollisionPlan {
                parent_id,
                source_collision_id: collision_id,
                shape,
            }),
            Err(reason) => {
                report.warnings.push(format!(
                    "Skyrim collision block {collision_id}: {reason}; stripped"
                ));
                report.stripped += 1;
            }
        }
        collect_collision_subtree(nif, collision_id, &mut remove);
    }

    for plan in &plans {
        if let Some(parent) = nif.blocks.get_mut(plan.parent_id) {
            parent.set_field("Collision Object", NifValue::Ref(-1));
        }
    }
    let removed_ids = remove.iter().copied().collect::<Vec<_>>();
    let remapped_parents = plans
        .iter()
        .map(|plan| remapped_id(plan.parent_id, &remove))
        .collect::<Vec<_>>();
    nif.remove_blocks(&removed_ids);

    for (plan, parent_id) in plans.into_iter().zip(remapped_parents) {
        match install_static_collision(nif, parent_id, plan.shape) {
            Ok(()) => report.converted += 1,
            Err(reason) => {
                report.warnings.push(format!(
                    "Skyrim collision block {}: FO4 static build failed ({reason}); stripped",
                    plan.source_collision_id
                ));
                report.stripped += 1;
            }
        }
    }
    report
}

fn validate_static_body(body: &NifBlock) -> Result<(), String> {
    if body.type_name != "bhkRigidBody" {
        return Err(format!(
            "unsupported animated/dynamic body type {}",
            body.type_name
        ));
    }
    let constraints = numeric(body.get_field("Num Constraints")).unwrap_or(0);
    if constraints != 0 {
        return Err("constrained rigid bodies are outside the static bridge".to_string());
    }
    let info = struct_field(
        body.fields
            .get("Rigid Body Info:2010")
            .or_else(|| body.get_field("Rigid Body Info")),
    )
    .ok_or_else(|| "missing Skyrim rigid-body info".to_string())?;
    let motion = numeric(info.get("Motion System")).unwrap_or(u64::MAX);
    let mass = float(info.get("Mass")).unwrap_or(1.0);
    if !matches!(motion, SOURCE_FIXED_MOTION | SOURCE_BOX_STABILIZED_MOTION)
        || mass.abs() > f32::EPSILON
    {
        return Err(format!(
            "non-static rigid body (motion={motion}, mass={mass}) is outside the static bridge"
        ));
    }
    Ok(())
}

fn decode_static_shape(nif: &NifFile, shape_id: usize) -> Result<MultiBodyShape, String> {
    let children = decode_children(nif, shape_id, identity_matrix(), &mut HashSet::new())?;
    if children.is_empty() {
        return Err("collision shape decoded to no geometry".to_string());
    }
    if children.len() == 1 {
        let child = children.into_iter().next().expect("single child");
        return Ok(match child.kind {
            CompoundChildKind::Polytope { vertices } => MultiBodyShape::Polytope { vertices },
            CompoundChildKind::SourcePolytope { shape } => MultiBodyShape::SourcePolytope { shape },
            CompoundChildKind::CompressedMesh {
                vertices,
                triangles,
            } => MultiBodyShape::CompressedMesh {
                vertices,
                triangles,
            },
        });
    }
    Ok(MultiBodyShape::Compound { children })
}

fn decode_children(
    nif: &NifFile,
    shape_id: usize,
    transform: [[f32; 4]; 4],
    visited: &mut HashSet<usize>,
) -> Result<Vec<CompoundChild>, String> {
    if !visited.insert(shape_id) {
        return Err(format!("collision shape cycle at block {shape_id}"));
    }
    let shape = nif
        .get_block(shape_id)
        .ok_or_else(|| format!("collision shape block {shape_id} is missing"))?;
    let result = match shape.type_name.as_str() {
        "bhkConvexVerticesShape" => {
            let vertices = vec4_array(shape.get_field("Vertices"));
            polytope_child(vertices, transform, shape_id)
        }
        "bhkBoxShape" => {
            let half = vec3(shape.get_field("Dimensions"))
                .ok_or_else(|| format!("bhkBoxShape {shape_id} has no dimensions"))?;
            polytope_child(box_vertices(half), transform, shape_id)
        }
        "bhkSphereShape" => {
            let radius = float(shape.get_field("Radius"))
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or_else(|| format!("bhkSphereShape {shape_id} has invalid radius"))?;
            polytope_child(sphere_vertices(radius), transform, shape_id)
        }
        "bhkCapsuleShape" => {
            let a = vec3(shape.get_field("First Point"))
                .ok_or_else(|| format!("bhkCapsuleShape {shape_id} has no first point"))?;
            let b = vec3(shape.get_field("Second Point"))
                .ok_or_else(|| format!("bhkCapsuleShape {shape_id} has no second point"))?;
            let radius = [
                float(shape.get_field("Radius")),
                float(shape.get_field("Radius 1")),
                float(shape.get_field("Radius 2")),
            ]
            .into_iter()
            .flatten()
            .filter(|value| value.is_finite() && *value > 0.0)
            .fold(0.0f32, f32::max);
            if radius <= 0.0 {
                return Err(format!("bhkCapsuleShape {shape_id} has invalid radius"));
            }
            polytope_child(capsule_vertices(a, b, radius), transform, shape_id)
        }
        "bhkListShape" => {
            let refs = ref_array(shape.get_field("Sub Shapes"));
            if refs.is_empty() {
                return Err(format!("bhkListShape {shape_id} is empty"));
            }
            let mut children = Vec::new();
            for child_id in refs {
                children.extend(decode_children(nif, child_id, transform, visited)?);
            }
            Ok(children)
        }
        "bhkTransformShape" | "bhkConvexTransformShape" => {
            let child_id = ref_field(shape, "Shape")
                .ok_or_else(|| format!("{} {shape_id} has no child", shape.type_name))?;
            let local = matrix44(shape.get_field("Transform")).unwrap_or_else(identity_matrix);
            decode_children(nif, child_id, multiply_matrix(transform, local), visited)
        }
        "bhkMoppBvTreeShape" => {
            let child_id = ref_field(shape, "Shape")
                .ok_or_else(|| format!("bhkMoppBvTreeShape {shape_id} has no child"))?;
            let child = nif
                .get_block(child_id)
                .ok_or_else(|| format!("MOPP child block {child_id} is missing"))?;
            if child.type_name != "bhkCompressedMeshShape" {
                return Err(format!(
                    "unsupported Skyrim MOPP child {} at block {child_id}",
                    child.type_name
                ));
            }
            decode_children(nif, child_id, transform, visited)
        }
        "bhkCompressedMeshShape" => compressed_mesh_child(nif, shape, transform, shape_id),
        unsupported => Err(format!(
            "unsupported Skyrim static collision shape {unsupported} at block {shape_id}"
        )),
    };
    visited.remove(&shape_id);
    result
}

fn compressed_mesh_child(
    nif: &NifFile,
    shape: &NifBlock,
    transform: [[f32; 4]; 4],
    shape_id: usize,
) -> Result<Vec<CompoundChild>, String> {
    let data_id = ref_field(shape, "Data")
        .ok_or_else(|| format!("bhkCompressedMeshShape {shape_id} has no data"))?;
    let data = nif
        .get_block(data_id)
        .filter(|block| block.type_name == "bhkCompressedMeshShapeData")
        .ok_or_else(|| format!("compressed mesh data block {data_id} is missing or invalid"))?;
    let shape_scale = vec3(shape.get_field("Scale"))
        .filter(|scale| scale.iter().all(|value| value.is_finite()))
        .ok_or_else(|| format!("bhkCompressedMeshShape {shape_id} has an invalid scale"))?;
    let (mut vertices, triangles) = decode_compressed_mesh_data(data)?;
    for vertex in &mut vertices {
        *vertex = transform_point(
            transform,
            [
                vertex[0] * shape_scale[0],
                vertex[1] * shape_scale[1],
                vertex[2] * shape_scale[2],
            ],
        );
    }
    Ok(vec![CompoundChild {
        transform: identity_matrix(),
        kind: CompoundChildKind::CompressedMesh {
            vertices,
            triangles,
        },
    }])
}

fn decode_compressed_mesh_data(data: &NifBlock) -> Result<(Vec<[f32; 3]>, Vec<[u32; 3]>), String> {
    let big_vert_values = compressed_array(data.get_field("Big Verts"), "Big Verts")?;
    let mut vertices = Vec::with_capacity(big_vert_values.len());
    for (index, value) in big_vert_values.iter().enumerate() {
        vertices.push(
            finite_vec3(value)
                .ok_or_else(|| format!("compressed mesh Big Verts entry {index} is invalid"))?,
        );
    }
    let big_vertex_count = vertices.len();
    let mut triangles = Vec::new();
    for (index, value) in compressed_array(data.get_field("Big Tris"), "Big Tris")?
        .iter()
        .enumerate()
    {
        let fields = struct_value(value)
            .ok_or_else(|| format!("compressed mesh Big Tris entry {index} is invalid"))?;
        let triangle = strict_triangle(fields.get("Triangle").unwrap_or(value), "Big Tris")?;
        validate_local_triangle(triangle, big_vertex_count, "Big Tris")?;
        triangles.push(triangle);
    }
    let transforms = parse_chunk_transforms(data)?;
    let chunks = compressed_struct_array(data.get_field("Chunks"), "Chunks")?;

    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let reference = required_numeric(chunk.get("Reference"), "chunk Reference")?;
        let source = if reference != u16::MAX as u64 {
            let reference_index = usize::try_from(reference).map_err(|_| {
                format!("compressed mesh chunk {chunk_index} reference is too large")
            })?;
            chunks.get(reference_index).copied().ok_or_else(|| {
                format!("compressed mesh chunk {chunk_index} references missing chunk {reference}")
            })?
        } else {
            *chunk
        };
        let offset = vec3(source.get("Offset"))
            .filter(|offset| offset.iter().all(|value| value.is_finite()))
            .ok_or_else(|| format!("compressed mesh chunk {chunk_index} has an invalid offset"))?;
        let transform_index = usize::try_from(required_numeric(
            chunk.get("Transform Index"),
            "chunk Transform Index",
        )?)
        .map_err(|_| format!("compressed mesh chunk {chunk_index} transform index is too large"))?;
        let (translation, rotation) = transforms.get(transform_index).copied().ok_or_else(|| {
            format!(
                "compressed mesh chunk {chunk_index} references missing transform {transform_index}"
            )
        })?;
        let base = u32::try_from(vertices.len())
            .map_err(|_| "compressed mesh has too many vertices".to_string())?;
        let chunk_vertices = compressed_array(source.get("Vertices"), "chunk Vertices")?;
        for (vertex_index, value) in chunk_vertices.iter().enumerate() {
            let encoded = finite_vec3(value).ok_or_else(|| {
                format!("compressed mesh chunk {chunk_index} vertex {vertex_index} is invalid")
            })?;
            let local = [
                encoded[0] / 1000.0 + offset[0],
                encoded[1] / 1000.0 + offset[1],
                encoded[2] / 1000.0 + offset[2],
            ];
            vertices.push(add(rotate_quaternion(local, rotation), translation));
        }

        let local_vertex_count = chunk_vertices.len();
        let indices = compressed_array(source.get("Indices"), "chunk Indices")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let local = required_u32(Some(value), "chunk index")?;
                if local as usize >= local_vertex_count {
                    return Err(format!(
                        "compressed mesh chunk {chunk_index} index {index} ({local}) exceeds local vertex count {local_vertex_count}"
                    ));
                }
                Ok(local)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let strip_lengths = compressed_array(source.get("Strip Lengths"), "chunk Strip Lengths")?
            .iter()
            .map(|value| {
                usize::try_from(required_numeric(Some(value), "chunk strip length")?)
                    .map_err(|_| "compressed mesh chunk strip length is too large".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut start = 0usize;
        for length in strip_lengths {
            let end = start.checked_add(length).ok_or_else(|| {
                format!("compressed mesh chunk {chunk_index} strip length overflow")
            })?;
            let strip = indices.get(start..end).ok_or_else(|| {
                format!("compressed mesh chunk {chunk_index} has a truncated strip")
            })?;
            for index in 0..strip.len().saturating_sub(2) {
                let triangle = if index % 2 == 0 {
                    [strip[index], strip[index + 1], strip[index + 2]]
                } else {
                    [strip[index], strip[index + 2], strip[index + 1]]
                };
                triangles.push(offset_triangle(base, triangle, chunk_index)?);
            }
            start = end;
        }
        let triangle_indices = &indices[start..];
        if triangle_indices.len() % 3 != 0 {
            return Err(format!(
                "compressed mesh chunk {chunk_index} has a truncated triangle list"
            ));
        }
        triangles.extend(
            triangle_indices
                .chunks_exact(3)
                .map(|tri| offset_triangle(base, [tri[0], tri[1], tri[2]], chunk_index))
                .collect::<Result<Vec<_>, String>>()?,
        );
    }

    if vertices.len() < 3 || triangles.is_empty() {
        return Err("compressed mesh decoded to no triangle geometry".to_string());
    }
    if triangles
        .iter()
        .flatten()
        .any(|index| *index as usize >= vertices.len())
    {
        return Err("compressed mesh contains an out-of-range triangle index".to_string());
    }
    Ok((vertices, triangles))
}

fn parse_chunk_transforms(data: &NifBlock) -> Result<Vec<([f32; 3], [f32; 4])>, String> {
    compressed_struct_array(data.get_field("Chunk Transforms"), "Chunk Transforms")?
        .into_iter()
        .enumerate()
        .map(|(index, fields)| {
            let translation = vec3(fields.get("Translation"))
                .filter(|value| value.iter().all(|component| component.is_finite()))
                .ok_or_else(|| {
                    format!("compressed mesh transform {index} has invalid translation")
                })?;
            let rotation = quaternion(fields.get("Rotation"))
                .and_then(normalize_quaternion)
                .ok_or_else(|| format!("compressed mesh transform {index} has invalid rotation"))?;
            Ok((translation, rotation))
        })
        .collect()
}

fn compressed_array<'a>(
    value: Option<&'a NifValue>,
    field_name: &str,
) -> Result<&'a [NifValue], String> {
    match value {
        Some(NifValue::Array(values)) => Ok(values),
        _ => Err(format!(
            "compressed mesh field {field_name} is not an array"
        )),
    }
}

fn compressed_struct_array<'a>(
    value: Option<&'a NifValue>,
    field_name: &str,
) -> Result<Vec<&'a indexmap::IndexMap<String, NifValue>>, String> {
    compressed_array(value, field_name)?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            struct_value(value).ok_or_else(|| {
                format!("compressed mesh field {field_name} entry {index} is not a struct")
            })
        })
        .collect()
}

fn strict_triangle(value: &NifValue, field_name: &str) -> Result<[u32; 3], String> {
    let fields = struct_value(value)
        .ok_or_else(|| format!("compressed mesh field {field_name} has an invalid triangle"))?;
    Ok([
        required_u32(fields.get("v1"), field_name)?,
        required_u32(fields.get("v2"), field_name)?,
        required_u32(fields.get("v3"), field_name)?,
    ])
}

fn validate_local_triangle(
    triangle: [u32; 3],
    vertex_count: usize,
    field_name: &str,
) -> Result<(), String> {
    if let Some(index) = triangle
        .into_iter()
        .find(|index| *index as usize >= vertex_count)
    {
        return Err(format!(
            "compressed mesh {field_name} index {index} exceeds local vertex count {vertex_count}"
        ));
    }
    Ok(())
}

fn offset_triangle(base: u32, triangle: [u32; 3], chunk_index: usize) -> Result<[u32; 3], String> {
    Ok([
        base.checked_add(triangle[0]).ok_or_else(|| {
            format!("compressed mesh chunk {chunk_index} triangle index overflow")
        })?,
        base.checked_add(triangle[1]).ok_or_else(|| {
            format!("compressed mesh chunk {chunk_index} triangle index overflow")
        })?,
        base.checked_add(triangle[2]).ok_or_else(|| {
            format!("compressed mesh chunk {chunk_index} triangle index overflow")
        })?,
    ])
}

fn required_numeric(value: Option<&NifValue>, field_name: &str) -> Result<u64, String> {
    numeric(value).ok_or_else(|| format!("compressed mesh field {field_name} is invalid"))
}

fn required_u32(value: Option<&NifValue>, field_name: &str) -> Result<u32, String> {
    u32::try_from(required_numeric(value, field_name)?)
        .map_err(|_| format!("compressed mesh field {field_name} exceeds u32"))
}

fn finite_vec3(value: &NifValue) -> Option<[f32; 3]> {
    vec3(Some(value)).filter(|vector| vector.iter().all(|component| component.is_finite()))
}

fn struct_value(value: &NifValue) -> Option<&indexmap::IndexMap<String, NifValue>> {
    match value {
        NifValue::Struct(fields) => Some(fields),
        _ => None,
    }
}

fn polytope_child(
    vertices: Vec<[f32; 3]>,
    transform: [[f32; 4]; 4],
    shape_id: usize,
) -> Result<Vec<CompoundChild>, String> {
    if vertices.len() < 4 || vertices.iter().flatten().any(|value| !value.is_finite()) {
        return Err(format!(
            "collision shape {shape_id} has fewer than four finite vertices"
        ));
    }
    Ok(vec![CompoundChild {
        transform: identity_matrix(),
        kind: CompoundChildKind::Polytope {
            vertices: vertices
                .into_iter()
                .map(|vertex| transform_point(transform, vertex))
                .collect(),
        },
    }])
}

fn install_static_collision(
    nif: &mut NifFile,
    parent_id: usize,
    shape: MultiBodyShape,
) -> Result<(), String> {
    if nif.get_block(parent_id).is_none() {
        return Err(format!("parent block {parent_id} is missing"));
    }
    let options = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: FO4_STATIC_LAYER,
        mass: 0.0,
        convex_radius: DEFAULT_COLLISION_RADIUS,
        materials: Vec::new(),
        user_data: None,
        body_props_raw: None,
        mass_distribution: None,
    };
    let metadata = [BodyMeta {
        layer: FO4_STATIC_LAYER,
        motion_type: BodyMotionType::Static,
        ..BodyMeta::default()
    }];
    let blob = build_fo4_multi_body_collision(&[shape], &options, None, Some(&metadata))
        .map_err(|error| error.to_string())?;
    let summary = havok_native::api::havok_collision_summary(&blob)
        .map_err(|error| format!("FO4 collision validation failed: {error}"))?;
    if summary.contains("\"n_vertices\":0") || summary.contains("\"n_instances\":0") {
        return Err("FO4 collision validation found an empty shape".to_string());
    }

    let mut physics_fields = indexmap::IndexMap::new();
    physics_fields.insert(
        "Binary Data".to_string(),
        crate::cloth::bytes_to_byte_array(&blob),
    );
    let physics_id = nif.add_block("bhkPhysicsSystem", Some(physics_fields));

    let mut collision_fields = indexmap::IndexMap::new();
    collision_fields.insert("Flags".to_string(), NifValue::UInt(0x80));
    collision_fields.insert("Target".to_string(), NifValue::Ref(parent_id as i32));
    collision_fields.insert("Data".to_string(), NifValue::Ref(physics_id as i32));
    collision_fields.insert("Body ID".to_string(), NifValue::UInt(0));
    let collision_id = nif.add_block("bhkNPCollisionObject", Some(collision_fields));
    nif.blocks[parent_id].set_field("Collision Object", NifValue::Ref(collision_id as i32));
    Ok(())
}

fn collect_collision_subtree(nif: &NifFile, block_id: usize, output: &mut HashSet<usize>) {
    if !output.insert(block_id) {
        return;
    }
    let Some(block) = nif.get_block(block_id) else {
        return;
    };
    let fields: &[&str] = match block.type_name.as_str() {
        "bhkCollisionObject" => &["Body"],
        "bhkRigidBody" | "bhkRigidBodyT" => &["Shape", "Constraints"],
        "bhkListShape" => &["Sub Shapes"],
        "bhkTransformShape" | "bhkConvexTransformShape" | "bhkMoppBvTreeShape" => &["Shape"],
        "bhkCompressedMeshShape" => &["Data"],
        _ => &[],
    };
    for field in fields {
        if *field == "Constraints" {
            for child in ref_array(block.get_field(field)) {
                collect_collision_subtree(nif, child, output);
            }
        } else if *field == "Sub Shapes" {
            for child in ref_array(block.get_field(field)) {
                collect_collision_subtree(nif, child, output);
            }
        } else if let Some(child) = ref_field(block, field) {
            collect_collision_subtree(nif, child, output);
        }
    }
}

fn remapped_id(old_id: usize, removed: &HashSet<usize>) -> usize {
    old_id - removed.iter().filter(|id| **id < old_id).count()
}

fn box_vertices(half: [f32; 3]) -> Vec<[f32; 3]> {
    let [x, y, z] = half;
    vec![
        [-x, -y, -z],
        [x, -y, -z],
        [-x, y, -z],
        [x, y, -z],
        [-x, -y, z],
        [x, -y, z],
        [-x, y, z],
        [x, y, z],
    ]
}

fn sphere_vertices(radius: f32) -> Vec<[f32; 3]> {
    vec![
        [radius, 0.0, 0.0],
        [-radius, 0.0, 0.0],
        [0.0, radius, 0.0],
        [0.0, -radius, 0.0],
        [0.0, 0.0, radius],
        [0.0, 0.0, -radius],
    ]
}

fn capsule_vertices(a: [f32; 3], b: [f32; 3], radius: f32) -> Vec<[f32; 3]> {
    let axis = normalize(subtract(b, a)).unwrap_or([0.0, 0.0, 1.0]);
    let seed = if axis[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let tangent = normalize(cross(axis, seed)).unwrap_or([1.0, 0.0, 0.0]);
    let bitangent = normalize(cross(axis, tangent)).unwrap_or([0.0, 1.0, 0.0]);
    let mut vertices = Vec::with_capacity(18);
    for center in [a, b] {
        for index in 0..8 {
            let angle = index as f32 * std::f32::consts::TAU / 8.0;
            vertices.push(add(
                center,
                add(
                    scale(tangent, radius * angle.cos()),
                    scale(bitangent, radius * angle.sin()),
                ),
            ));
        }
    }
    vertices.push(subtract(a, scale(axis, radius)));
    vertices.push(add(b, scale(axis, radius)));
    vertices
}

fn vec4_array(value: Option<&NifValue>) -> Vec<[f32; 3]> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(|value| vec3(Some(value)))
            .collect(),
        _ => Vec::new(),
    }
}

fn vec3(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value {
        Some(NifValue::Vec3(value)) => Some(*value),
        Some(NifValue::Vec4(value)) => Some([value[0], value[1], value[2]]),
        Some(NifValue::Struct(fields)) => Some([
            float(fields.get("x"))?,
            float(fields.get("y"))?,
            float(fields.get("z"))?,
        ]),
        _ => None,
    }
}

fn matrix44(value: Option<&NifValue>) -> Option<[[f32; 4]; 4]> {
    match value {
        Some(NifValue::Matrix44(value)) => Some(*value),
        Some(NifValue::Struct(fields)) => Some([
            [
                float(fields.get("m11"))?,
                float(fields.get("m21"))?,
                float(fields.get("m31"))?,
                float(fields.get("m41"))?,
            ],
            [
                float(fields.get("m12"))?,
                float(fields.get("m22"))?,
                float(fields.get("m32"))?,
                float(fields.get("m42"))?,
            ],
            [
                float(fields.get("m13"))?,
                float(fields.get("m23"))?,
                float(fields.get("m33"))?,
                float(fields.get("m43"))?,
            ],
            [
                float(fields.get("m14"))?,
                float(fields.get("m24"))?,
                float(fields.get("m34"))?,
                float(fields.get("m44"))?,
            ],
        ]),
        _ => None,
    }
}

fn quaternion(value: Option<&NifValue>) -> Option<[f32; 4]> {
    match value {
        Some(NifValue::Quaternion(value)) | Some(NifValue::Vec4(value)) => Some(*value),
        Some(NifValue::Struct(fields)) => Some([
            float(fields.get("x"))?,
            float(fields.get("y"))?,
            float(fields.get("z"))?,
            float(fields.get("w"))?,
        ]),
        _ => None,
    }
}

fn normalize_quaternion(value: [f32; 4]) -> Option<[f32; 4]> {
    if value.iter().any(|component| !component.is_finite()) {
        return None;
    }
    let length_squared = value
        .iter()
        .map(|component| component * component)
        .sum::<f32>();
    if length_squared <= f32::EPSILON {
        return None;
    }
    let inverse_length = length_squared.sqrt().recip();
    Some([
        value[0] * inverse_length,
        value[1] * inverse_length,
        value[2] * inverse_length,
        value[3] * inverse_length,
    ])
}

fn rotate_quaternion(point: [f32; 3], quaternion: [f32; 4]) -> [f32; 3] {
    let vector = [quaternion[0], quaternion[1], quaternion[2]];
    let uv = cross(vector, point);
    let uuv = cross(vector, uv);
    add(point, add(scale(uv, 2.0 * quaternion[3]), scale(uuv, 2.0)))
}

fn identity_matrix() -> [[f32; 4]; 4] {
    CompoundChild::identity_transform()
}

fn multiply_matrix(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut output = [[0.0; 4]; 4];
    for row in 0..4 {
        for col in 0..4 {
            output[row][col] = (0..4)
                .map(|index| left[row][index] * right[index][col])
                .sum();
        }
    }
    output
}

fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * point[0] + matrix[0][1] * point[1] + matrix[0][2] * point[2] + matrix[0][3],
        matrix[1][0] * point[0] + matrix[1][1] * point[1] + matrix[1][2] * point[2] + matrix[1][3],
        matrix[2][0] * point[0] + matrix[2][1] * point[1] + matrix[2][2] * point[2] + matrix[2][3],
    ]
}

fn ref_field(block: &NifBlock, name: &str) -> Option<usize> {
    let value = block.get_field(name)?;
    let id = value.as_i64();
    (id >= 0).then_some(id as usize)
}

fn ref_array(value: Option<&NifValue>) -> Vec<usize> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(|value| {
                let id = value.as_i64();
                (id >= 0).then_some(id as usize)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn struct_field(value: Option<&NifValue>) -> Option<&indexmap::IndexMap<String, NifValue>> {
    match value {
        Some(NifValue::Struct(fields)) => Some(fields),
        _ => None,
    }
}

fn numeric(value: Option<&NifValue>) -> Option<u64> {
    match value {
        Some(NifValue::UInt(value)) => Some(*value),
        Some(NifValue::Int(value)) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn float(value: Option<&NifValue>) -> Option<f32> {
    match value {
        Some(NifValue::Float(value)) => Some(*value as f32),
        Some(NifValue::Int(value)) => Some(*value as f32),
        Some(NifValue::UInt(value)) => Some(*value as f32),
        _ => None,
    }
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(value: [f32; 3], factor: f32) -> [f32; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    (length > f32::EPSILON).then(|| scale(value, length.recip()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    fn fixed_body_info() -> NifValue {
        NifValue::Struct(IndexMap::from([
            (
                "Motion System".to_string(),
                NifValue::UInt(SOURCE_FIXED_MOTION),
            ),
            ("Mass".to_string(), NifValue::Float(0.0)),
        ]))
    }

    fn attach_fixed_collision(nif: &mut NifFile, shape_id: usize) {
        let body_id = nif.add_block(
            "bhkRigidBody",
            Some(IndexMap::from([
                ("Shape".to_string(), NifValue::Ref(shape_id as i32)),
                ("Rigid Body Info:2010".to_string(), fixed_body_info()),
                ("Num Constraints".to_string(), NifValue::UInt(0)),
            ])),
        );
        let collision_id = nif.add_block(
            "bhkCollisionObject",
            Some(IndexMap::from([
                ("Target".to_string(), NifValue::Ref(0)),
                ("Body".to_string(), NifValue::Ref(body_id as i32)),
            ])),
        );
        nif.blocks[0].set_field("Collision Object", NifValue::Ref(collision_id as i32));
    }

    fn physics_blob(nif: &NifFile) -> Vec<u8> {
        let physics = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "bhkPhysicsSystem")
            .expect("physics system");
        crate::cloth::byte_array_to_bytes(physics.get_field("Binary Data").expect("binary data"))
            .expect("blob")
    }

    fn encoded_vertex(x: u64, y: u64, z: u64) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("x".to_string(), NifValue::UInt(x)),
            ("y".to_string(), NifValue::UInt(y)),
            ("z".to_string(), NifValue::UInt(z)),
        ]))
    }

    fn chunk_transform(translation: [f32; 4], rotation: [f32; 4]) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("Translation".to_string(), NifValue::Vec4(translation)),
            ("Rotation".to_string(), NifValue::Quaternion(rotation)),
        ]))
    }

    fn compressed_chunk(
        reference: u64,
        transform_index: u64,
        vertices: Vec<NifValue>,
        indices: Vec<NifValue>,
        strip_lengths: Vec<NifValue>,
    ) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("Offset".to_string(), NifValue::Vec4([0.0, 0.0, 0.0, 0.0])),
            ("Reference".to_string(), NifValue::UInt(reference)),
            (
                "Transform Index".to_string(),
                NifValue::UInt(transform_index),
            ),
            ("Vertices".to_string(), NifValue::Array(vertices)),
            ("Indices".to_string(), NifValue::Array(indices)),
            ("Strip Lengths".to_string(), NifValue::Array(strip_lengths)),
        ]))
    }

    fn compressed_data_fields(
        big_verts: Vec<NifValue>,
        big_tris: Vec<NifValue>,
        transforms: Vec<NifValue>,
        chunks: Vec<NifValue>,
    ) -> IndexMap<String, NifValue> {
        IndexMap::from([
            ("Big Verts".to_string(), NifValue::Array(big_verts)),
            ("Big Tris".to_string(), NifValue::Array(big_tris)),
            ("Chunk Transforms".to_string(), NifValue::Array(transforms)),
            ("Chunks".to_string(), NifValue::Array(chunks)),
        ])
    }

    fn big_triangle(v1: u64, v2: u64, v3: u64) -> NifValue {
        NifValue::Struct(IndexMap::from([(
            "Triangle".to_string(),
            NifValue::Struct(IndexMap::from([
                ("v1".to_string(), NifValue::UInt(v1)),
                ("v2".to_string(), NifValue::UInt(v2)),
                ("v3".to_string(), NifValue::UInt(v3)),
            ])),
        )]))
    }

    fn compressed_data_block(fields: IndexMap<String, NifValue>) -> NifBlock {
        let mut block = NifBlock::new(0, "bhkCompressedMeshShapeData");
        block.fields = fields;
        block
    }

    #[test]
    fn static_box_bridge_builds_fo4_np_collision() {
        let mut nif = NifFile::new("skyrimse");
        let mut box_fields = IndexMap::new();
        box_fields.insert("Dimensions".to_string(), NifValue::Vec3([1.0, 2.0, 3.0]));
        let shape_id = nif.add_block("bhkBoxShape", Some(box_fields));

        let mut body_fields = IndexMap::new();
        body_fields.insert("Shape".to_string(), NifValue::Ref(shape_id as i32));
        body_fields.insert("Rigid Body Info:2010".to_string(), fixed_body_info());
        body_fields.insert("Num Constraints".to_string(), NifValue::UInt(0));
        let body_id = nif.add_block("bhkRigidBody", Some(body_fields));

        let mut collision_fields = IndexMap::new();
        collision_fields.insert("Target".to_string(), NifValue::Ref(0));
        collision_fields.insert("Body".to_string(), NifValue::Ref(body_id as i32));
        let collision_id = nif.add_block("bhkCollisionObject", Some(collision_fields));
        nif.blocks[0].set_field("Collision Object", NifValue::Ref(collision_id as i32));

        let report = bridge_static_collision(&mut nif);
        assert_eq!(report.converted, 1, "{:?}", report.warnings);
        assert_eq!(report.stripped, 0);
        assert!(
            nif.blocks
                .iter()
                .any(|block| block.type_name == "bhkNPCollisionObject")
        );
        let physics = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "bhkPhysicsSystem")
            .expect("physics system");
        let bytes = crate::cloth::byte_array_to_bytes(
            physics.get_field("Binary Data").expect("binary data"),
        )
        .expect("blob");
        let summary = havok_native::api::havok_collision_summary(&bytes).expect("summary");
        assert!(summary.contains("hknpConvexPolytopeShape"));
    }

    #[test]
    fn dynamic_body_is_stripped_not_reinterpreted_as_static() {
        let mut nif = NifFile::new("skyrimse");
        let shape_id = nif.add_block("bhkSphereShape", None);
        nif.blocks[shape_id].set_field("Radius", NifValue::Float(1.0));
        let mut info = match fixed_body_info() {
            NifValue::Struct(fields) => fields,
            _ => unreachable!(),
        };
        info.insert("Motion System".to_string(), NifValue::UInt(1));
        info.insert("Mass".to_string(), NifValue::Float(2.0));
        let body_id = nif.add_block(
            "bhkRigidBody",
            Some(IndexMap::from([
                ("Shape".to_string(), NifValue::Ref(shape_id as i32)),
                ("Rigid Body Info:2010".to_string(), NifValue::Struct(info)),
                ("Num Constraints".to_string(), NifValue::UInt(0)),
            ])),
        );
        let collision_id = nif.add_block(
            "bhkCollisionObject",
            Some(IndexMap::from([
                ("Target".to_string(), NifValue::Ref(0)),
                ("Body".to_string(), NifValue::Ref(body_id as i32)),
            ])),
        );
        nif.blocks[0].set_field("Collision Object", NifValue::Ref(collision_id as i32));

        let report = bridge_static_collision(&mut nif);
        assert_eq!(report.converted, 0);
        assert_eq!(report.stripped, 1);
        assert!(report.warnings[0].contains("non-static rigid body"));
        assert!(
            !nif.blocks
                .iter()
                .any(|block| block.type_name == "bhkCollisionObject")
        );
    }

    #[test]
    fn mopp_collision_is_explicitly_unsupported() {
        let mut nif = NifFile::new("skyrimse");
        let box_id = nif.add_block("bhkBoxShape", None);
        let shape_id = nif.add_block(
            "bhkMoppBvTreeShape",
            Some(IndexMap::from([(
                "Shape".to_string(),
                NifValue::Ref(box_id as i32),
            )])),
        );
        let error = decode_static_shape(&nif, shape_id).expect_err("must reject MOPP");
        assert!(error.contains("unsupported Skyrim MOPP child"));
    }

    #[test]
    fn compressed_mesh_floor_builds_fo4_static_triangle_collision() {
        let mut nif = NifFile::new("skyrimse");
        let vertex = |x, y, z| {
            NifValue::Struct(IndexMap::from([
                ("x".to_string(), NifValue::UInt(x)),
                ("y".to_string(), NifValue::UInt(y)),
                ("z".to_string(), NifValue::UInt(z)),
            ]))
        };
        let data_id = nif.add_block(
            "bhkCompressedMeshShapeData",
            Some(IndexMap::from([
                ("Big Verts".to_string(), NifValue::Array(Vec::new())),
                ("Big Tris".to_string(), NifValue::Array(Vec::new())),
                (
                    "Chunk Transforms".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                        (
                            "Translation".to_string(),
                            NifValue::Vec4([0.0, 0.0, 0.0, 1.0]),
                        ),
                        (
                            "Rotation".to_string(),
                            NifValue::Quaternion([0.0, 0.0, 0.0, 1.0]),
                        ),
                    ]))]),
                ),
                (
                    "Chunks".to_string(),
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                        ("Offset".to_string(), NifValue::Vec4([-2.0, -2.0, 0.0, 0.0])),
                        ("Reference".to_string(), NifValue::UInt(u16::MAX as u64)),
                        ("Transform Index".to_string(), NifValue::UInt(0)),
                        (
                            "Vertices".to_string(),
                            NifValue::Array(vec![
                                vertex(0, 0, 0),
                                vertex(4000, 0, 0),
                                vertex(4000, 4000, 0),
                                vertex(0, 4000, 0),
                            ]),
                        ),
                        (
                            "Indices".to_string(),
                            NifValue::Array(
                                vec![0, 1, 2, 0, 2, 3]
                                    .into_iter()
                                    .map(NifValue::UInt)
                                    .collect(),
                            ),
                        ),
                        (
                            "Strip Lengths".to_string(),
                            NifValue::Array(vec![NifValue::UInt(3)]),
                        ),
                    ]))]),
                ),
            ])),
        );
        let compressed_id = nif.add_block(
            "bhkCompressedMeshShape",
            Some(IndexMap::from([
                ("Data".to_string(), NifValue::Ref(data_id as i32)),
                ("Scale".to_string(), NifValue::Vec4([1.0, 1.0, 1.0, 0.0])),
            ])),
        );
        let mopp_id = nif.add_block(
            "bhkMoppBvTreeShape",
            Some(IndexMap::from([(
                "Shape".to_string(),
                NifValue::Ref(compressed_id as i32),
            )])),
        );
        attach_fixed_collision(&mut nif, mopp_id);

        let report = bridge_static_collision(&mut nif);
        assert_eq!(report.converted, 1, "{:?}", report.warnings);
        assert_eq!(report.stripped, 0);
        let decoded = havok_native::collision::parse_fo4_compressed_mesh(&physics_blob(&nif))
            .expect("decode FO4 compressed mesh");
        assert_eq!(
            decoded
                .sections
                .iter()
                .map(|section| section.triangles.len())
                .sum::<usize>(),
            2
        );
        assert!(decoded.sections.iter().all(|section| {
            section
                .vertices
                .iter()
                .all(|vertex| vertex[2].abs() < 0.001)
        }));
    }

    #[test]
    fn compressed_mesh_references_apply_transforms_scales_and_strip_winding() {
        let mut nif = NifFile::new("skyrimse");
        let source_vertices = vec![
            encoded_vertex(0, 0, 0),
            encoded_vertex(1000, 0, 0),
            encoded_vertex(0, 1000, 0),
            encoded_vertex(1000, 1000, 0),
        ];
        let data_id = nif.add_block(
            "bhkCompressedMeshShapeData",
            Some(compressed_data_fields(
                Vec::new(),
                Vec::new(),
                vec![
                    chunk_transform([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
                    chunk_transform([10.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
                ],
                vec![
                    compressed_chunk(
                        u16::MAX as u64,
                        0,
                        source_vertices,
                        vec![0, 1, 2, 3].into_iter().map(NifValue::UInt).collect(),
                        vec![NifValue::UInt(4)],
                    ),
                    compressed_chunk(0, 1, Vec::new(), Vec::new(), Vec::new()),
                ],
            )),
        );
        let shape_id = nif.add_block(
            "bhkCompressedMeshShape",
            Some(IndexMap::from([
                ("Data".to_string(), NifValue::Ref(data_id as i32)),
                ("Scale".to_string(), NifValue::Vec4([2.0, 3.0, 1.0, 0.0])),
            ])),
        );

        let children = decode_children(&nif, shape_id, identity_matrix(), &mut HashSet::new())
            .expect("decode referenced chunks");
        let CompoundChildKind::CompressedMesh {
            vertices,
            triangles,
        } = &children[0].kind
        else {
            panic!("expected compressed mesh")
        };
        assert_eq!(vertices.len(), 8);
        assert_eq!(triangles, &vec![[0, 1, 2], [1, 3, 2], [4, 5, 6], [5, 7, 6]]);
        assert_eq!(vertices[1], [2.0, 0.0, 0.0]);
        assert_eq!(vertices[2], [0.0, 3.0, 0.0]);
        assert_eq!(vertices[4], [20.0, 0.0, 0.0]);
    }

    #[test]
    fn compressed_mesh_rejects_big_and_chunk_local_index_overruns() {
        let big_data = compressed_data_block(compressed_data_fields(
            vec![
                NifValue::Vec4([0.0, 0.0, 0.0, 0.0]),
                NifValue::Vec4([1.0, 0.0, 0.0, 0.0]),
                NifValue::Vec4([0.0, 1.0, 0.0, 0.0]),
            ],
            vec![big_triangle(0, 1, 3)],
            Vec::new(),
            Vec::new(),
        ));
        assert!(
            decode_compressed_mesh_data(&big_data)
                .expect_err("Big Tris must stay within Big Verts")
                .contains("Big Tris index 3")
        );

        let chunk_data = compressed_data_block(compressed_data_fields(
            Vec::new(),
            Vec::new(),
            vec![chunk_transform([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0])],
            vec![
                compressed_chunk(
                    u16::MAX as u64,
                    0,
                    vec![
                        encoded_vertex(0, 0, 0),
                        encoded_vertex(1000, 0, 0),
                        encoded_vertex(0, 1000, 0),
                    ],
                    vec![0, 1, 2].into_iter().map(NifValue::UInt).collect(),
                    Vec::new(),
                ),
                compressed_chunk(
                    u16::MAX as u64,
                    0,
                    vec![encoded_vertex(0, 0, 0)],
                    vec![0, 1, 2].into_iter().map(NifValue::UInt).collect(),
                    Vec::new(),
                ),
                compressed_chunk(
                    u16::MAX as u64,
                    0,
                    vec![
                        encoded_vertex(0, 0, 0),
                        encoded_vertex(1000, 0, 0),
                        encoded_vertex(0, 1000, 0),
                    ],
                    vec![0, 1, 2].into_iter().map(NifValue::UInt).collect(),
                    Vec::new(),
                ),
            ],
        ));
        assert!(
            decode_compressed_mesh_data(&chunk_data)
                .expect_err("chunk indices must stay within their source vertices")
                .contains("exceeds local vertex count 1")
        );
    }

    #[test]
    fn compressed_mesh_rejects_malformed_arrays_and_transform_indices() {
        let malformed = compressed_data_block(IndexMap::from([
            ("Big Verts".to_string(), NifValue::UInt(0)),
            ("Big Tris".to_string(), NifValue::Array(Vec::new())),
            ("Chunk Transforms".to_string(), NifValue::Array(Vec::new())),
            ("Chunks".to_string(), NifValue::Array(Vec::new())),
        ]));
        assert!(
            decode_compressed_mesh_data(&malformed)
                .expect_err("malformed arrays must not be ignored")
                .contains("Big Verts is not an array")
        );

        let missing_transform = compressed_data_block(compressed_data_fields(
            Vec::new(),
            Vec::new(),
            vec![chunk_transform([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0])],
            vec![compressed_chunk(
                u16::MAX as u64,
                1,
                vec![
                    encoded_vertex(0, 0, 0),
                    encoded_vertex(1000, 0, 0),
                    encoded_vertex(0, 1000, 0),
                ],
                vec![0, 1, 2].into_iter().map(NifValue::UInt).collect(),
                Vec::new(),
            )],
        ));
        assert!(
            decode_compressed_mesh_data(&missing_transform)
                .expect_err("missing transform must be rejected")
                .contains("references missing transform 1")
        );
    }

    #[test]
    fn real_skyrim_compressed_mesh_converts_when_fixture_is_available() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extracted/skyrimse/Meshes/architecture/winterhold/winterholdtowerintfloor02.nif");
        if !path.is_file() {
            return;
        }
        let mut nif = NifFile::load(path).expect("load Skyrim compressed mesh fixture");
        let report = bridge_static_collision(&mut nif);
        assert_eq!(report.converted, 1, "{:?}", report.warnings);
        assert_eq!(report.stripped, 0, "{:?}", report.warnings);
        let decoded = havok_native::collision::parse_fo4_compressed_mesh(&physics_blob(&nif))
            .expect("decode converted fixture collision");
        assert_eq!(
            decoded
                .sections
                .iter()
                .map(|section| section.triangles.len())
                .sum::<usize>(),
            120
        );
    }
}
