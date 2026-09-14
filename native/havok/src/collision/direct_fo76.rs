use std::collections::HashSet;

use super::compound::{
    SHAPE_INST_DEPRECATED, SHAPE_INST_HAS_ROTATION, SHAPE_INST_HAS_SCALE,
    SHAPE_INST_HAS_TRANSLATION, SHAPE_INST_IS_ENABLED, SHAPE_INST_SCALE_SURFACE, pack_inst_row_w,
};
use super::fo76_material::remap_fo76_collision_material_for_fo4;
use super::preview::{
    extract_preview_meshes_from_hkx, extract_raw_compressed_meshes_from_blob,
    extract_raw_compressed_meshes_from_hkx,
};
use super::validate::{Invariants, Severity, validate_objects};
use crate::error::{HavokError, HavokResult};
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::{HkxValue, f32_to_half};

const FO76_TAGFILE_VERSION: &str = "hk_2015.1.0-r1";
const FO4_PACKFILE_VERSION: &str = "hk_2014.1.0-r1";
const INVALID_ID: u32 = 0x7fff_ffff;
const FO76_COMPRESSED_MESH_FLAGS: u16 = 0x0004;
const FO76_COMPRESSED_MESH_DISPATCH_TYPE: u8 = 3;
const FO4_POLYTOPE_FLAGS: u16 = 0x0143;
const FO4_POLYTOPE_DISPATCH_TYPE: u8 = 1;
const FO4_POLYTOPE_SENTINEL_PLANES: [[f32; 4]; 2] = [[0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]];
const FO76_CAPSULE_FLAGS: u16 = 0x01C3;
const FO76_CAPSULE_DISPATCH_TYPE: u8 = 2;
const FO4_CAPSULE_FLAGS: u16 = 0x01C3;
const FO4_CAPSULE_DISPATCH_TYPE: u8 = 1;
const FO4_CAPSULE_SENTINEL_PLANE: [f32; 4] = [0.0, 0.0, 0.0, -3.402_823_5e38];
const FO4_COMPRESSED_MESH_FLAGS: u16 = 0x0204;
const FO4_COMPRESSED_MESH_DISPATCH_TYPE: u8 = 2;
const FO4_COMPOUND_FLAGS: u16 = 0x0004;
const FO4_COMPOUND_DISPATCH_TYPE: u8 = 2;
const FO4_CAPSULE_BASE_SIZE: usize = 0x70;
const FO4_COMPRESSED_MESH_INSTANCE_SIZE: usize = 0x90;
const INT24_MASK: u32 = 0x00ff_ffff;
const INT24_PREFIX: u32 = 0x3f00_0000;

const SUPPORTED_SOURCE_CLASSES: &[&str] = &[
    "hknpPhysicsSystemData",
    "hknpCompressedMeshShape",
    "hknpCompoundShape",
    "hkRefCountedProperties",
    "hknpCompressedMeshShapeData",
    "hknpDynamicCompoundShapeData",
    "hknpConvexPolytopeShape",
    "hknpCapsuleShape",
    "hknpBSMaterialProperties",
    "hknpConvexPolytopeShape::Connectivity",
    "hknpShapeMassProperties",
];

const HALF_MATERIAL_MEMBERS: &[&str] = &[
    "dynamicFriction",
    "staticFriction",
    "restitution",
    "weldingTolerance",
    "massChangerHeavyObjectFactor",
    "softContactForceFactor",
    "softContactDampFactor",
    "disablingCollisionsBetweenCvxCvxDynamicObjectsDistance",
];

#[derive(Debug, PartialEq)]
struct PhysicsSignature {
    materials: String,
    bodies: Vec<BodySignature>,
}

#[derive(Debug, PartialEq)]
struct BodySignature {
    semantics: Vec<(String, String)>,
    shape: ShapeSignature,
}

#[derive(Debug, PartialEq)]
struct ShapeSignature {
    class_name: String,
    semantics: Vec<(String, String)>,
    properties: Option<String>,
    instances: Vec<InstanceSignature>,
}

#[derive(Debug, PartialEq)]
struct InstanceSignature {
    semantics: Vec<(String, String)>,
    shape: Box<ShapeSignature>,
}

/// Transcode the static hknp collision graph embedded in a FO76 NIF to a
/// FO4/Havok 2014 packfile without reconstructing its mesh or compound leaves.
///
/// This deliberately accepts only a verified static graph subset. Unsupported
/// roots, dynamic bodies, constraints, nested compounds, or any semantic drift
/// during target serialization return an error. Non-dynamic source bodies can
/// carry stale keyframed/dynamic motion metadata; the static caller is
/// responsible for proving those bodies are not animated before this function
/// normalizes them. The caller also remains responsible for replacing the NIF
/// payload.
pub fn convert_fo76_embedded_static_collision_direct(blob: &[u8]) -> HavokResult<Vec<u8>> {
    if blob.len() < 8 || &blob[4..8] != b"TAG0" {
        return Err(direct_error("input is not an embedded TAG0 tagfile"));
    }

    let mut source = HkxFile::read(blob)?;
    if source.contents_version() != FO76_TAGFILE_VERSION {
        return Err(direct_error(format!(
            "expected {FO76_TAGFILE_VERSION}, found {}",
            source.contents_version()
        )));
    }

    validate_supported_source(&source)?;
    let source_signature = physics_signature(&source)?;
    let source_raw_meshes = extract_raw_compressed_meshes_from_hkx(&source, None);

    normalize_static_bodies(&mut source)?;
    normalize_compressed_mesh_shape_headers(&mut source);
    normalize_polytopes(&mut source)?;
    normalize_capsules(&mut source)?;
    normalize_compounds(&mut source)?;
    normalize_collision_materials(&mut source);
    normalize_compressed_mass_properties(&mut source);
    strip_connectivity(&mut source);
    source.set_class_version(11);
    source.set_contents_version(FO4_PACKFILE_VERSION);
    let expected_signature = physics_signature(&source)?;
    let expected_raw_meshes = extract_raw_compressed_meshes_from_hkx(&source, None);
    let expected_previews = body_previews(&source, source_signature.bodies.len());

    let mut output = source.save();
    let flat_convex_markers = source_raw_meshes
        .iter()
        .map(|mesh| mesh.primitive_stores_is_flat_convex)
        .collect::<Vec<_>>();
    super::compressed_mesh::patch_fo4_compressed_mesh_flat_convex_markers(
        &mut output,
        &flat_convex_markers,
    )?;
    let converted = HkxFile::read(&output)?;
    validate_converted(
        &output,
        &converted,
        &expected_signature,
        &expected_raw_meshes,
        &expected_previews,
    )?;
    Ok(output)
}

fn validate_supported_source(hkx: &HkxFile) -> HavokResult<()> {
    for object in hkx.objects() {
        if !SUPPORTED_SOURCE_CLASSES.contains(&object.class_name.as_str()) {
            return Err(direct_error(format!(
                "unsupported source class {}",
                object.class_name
            )));
        }
    }

    let physics_systems: Vec<_> = hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpPhysicsSystemData")
        .collect();
    if physics_systems.len() != 1 {
        return Err(direct_error(format!(
            "expected one hknpPhysicsSystemData, found {}",
            physics_systems.len()
        )));
    }
    let physics_system = physics_systems[0];
    require_empty_array_if_present(physics_system, "motionProperties")?;
    require_empty_array_if_present(physics_system, "motionCinfos")?;
    require_empty_array_if_present(physics_system, "constraintCinfos")?;

    let bodies = member_array(physics_system, "bodyCinfos")?;
    if bodies.is_empty() {
        return Err(direct_error("physics system has no bodies"));
    }

    let mut root_shapes = HashSet::new();
    let mut compound_roots = HashSet::new();
    for (body_index, body) in bodies.iter().enumerate() {
        let members = object_members(body)
            .ok_or_else(|| direct_error(format!("body {body_index} is not an inline object")))?;
        let flags = required_integer(members, "flags", &format!("body {body_index}"))?;
        if flags & 128 != 0 {
            return Err(direct_error(format!("body {body_index} is dynamic")));
        }
        if let Some(motion_type) = optional_integer(members, "motionType") {
            if !(0..=2).contains(&motion_type) {
                return Err(direct_error(format!(
                    "body {body_index} has unsupported motionType {motion_type}"
                )));
            }
        }
        let shape_index = required_pointer(members, "shape", &format!("body {body_index}"))?;
        let shape = hkx.objects().get(shape_index).ok_or_else(|| {
            direct_error(format!(
                "body {body_index} shape pointer {shape_index} is out of range"
            ))
        })?;
        match shape.class_name.as_str() {
            "hknpCompressedMeshShape" => validate_compressed_mesh(hkx, shape_index)?,
            "hknpConvexPolytopeShape" => validate_polytope(hkx, shape_index)?,
            "hknpCapsuleShape" => validate_capsule(hkx, shape_index)?,
            "hknpCompoundShape" => {
                validate_compound(hkx, shape_index)?;
                compound_roots.insert(shape_index);
            }
            other => {
                return Err(direct_error(format!(
                    "body {body_index} has unsupported root shape {other}"
                )));
            }
        }
        root_shapes.insert(shape_index);
    }

    for (index, object) in hkx.objects().iter().enumerate() {
        if object.class_name == "hknpCompoundShape" && !compound_roots.contains(&index) {
            return Err(direct_error(format!(
                "compound shape {index} is not a body root"
            )));
        }
    }

    let referenced = member_array(physics_system, "referencedObjects")?;
    let referenced_roots: HashSet<_> = referenced
        .iter()
        .filter_map(|value| match value {
            HkxValue::Pointer(Some(index)) => Some(*index),
            _ => None,
        })
        .collect();
    if referenced_roots != root_shapes {
        return Err(direct_error(
            "referencedObjects does not match the body root shapes",
        ));
    }

    let compressed_shape_count = hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpCompressedMeshShape")
        .count();
    let raw_mesh_count = extract_raw_compressed_meshes_from_hkx(hkx, None).len();
    if compressed_shape_count != raw_mesh_count {
        return Err(direct_error(format!(
            "decoded {raw_mesh_count} of {compressed_shape_count} compressed meshes"
        )));
    }
    Ok(())
}

fn validate_compound(hkx: &HkxFile, shape_index: usize) -> HavokResult<()> {
    let shape = &hkx.objects()[shape_index];
    let data_index = required_pointer(
        &shape.members,
        "boundingVolumeData",
        &format!("compound {shape_index}"),
    )?;
    let data = hkx.objects().get(data_index).ok_or_else(|| {
        direct_error(format!(
            "compound {shape_index} boundingVolumeData pointer is out of range"
        ))
    })?;
    if data.class_name != "hknpDynamicCompoundShapeData" {
        return Err(direct_error(format!(
            "compound {shape_index} has unsupported backing class {}",
            data.class_name
        )));
    }

    let instances = member_object(&shape.members, "instances")
        .ok_or_else(|| direct_error(format!("compound {shape_index} has no instances object")))?;
    let elements = member_array_from_members(instances, "elements")
        .ok_or_else(|| direct_error(format!("compound {shape_index} has no instance elements")))?;
    if elements.is_empty() {
        return Err(direct_error(format!(
            "compound {shape_index} has no explicit instances"
        )));
    }
    if optional_integer(instances, "firstFree") != Some(-1) {
        return Err(direct_error(format!(
            "compound {shape_index} contains free-list holes"
        )));
    }

    for (instance_index, instance) in elements.iter().enumerate() {
        let members = object_members(instance).ok_or_else(|| {
            direct_error(format!(
                "compound {shape_index} instance {instance_index} is not an object"
            ))
        })?;
        required_floats(
            members,
            "transform",
            16,
            &format!("compound {shape_index} instance {instance_index}"),
        )?;
        required_floats(
            members,
            "scale",
            4,
            &format!("compound {shape_index} instance {instance_index}"),
        )?;
        required_integer(
            members,
            "shapeTag",
            &format!("compound {shape_index} instance {instance_index}"),
        )?;
        required_integer(
            members,
            "destructionTag",
            &format!("compound {shape_index} instance {instance_index}"),
        )?;
        let leaf_index = required_pointer(
            members,
            "shape",
            &format!("compound {shape_index} instance {instance_index}"),
        )?;
        let leaf = hkx.objects().get(leaf_index).ok_or_else(|| {
            direct_error(format!(
                "compound {shape_index} instance {instance_index} shape is out of range"
            ))
        })?;
        match leaf.class_name.as_str() {
            "hknpCompressedMeshShape" => validate_compressed_mesh(hkx, leaf_index)?,
            "hknpConvexPolytopeShape" => validate_polytope(hkx, leaf_index)?,
            "hknpCapsuleShape" => validate_capsule(hkx, leaf_index)?,
            other => {
                return Err(direct_error(format!(
                    "compound {shape_index} instance {instance_index} has unsupported leaf {other}"
                )));
            }
        }
    }
    Ok(())
}

fn validate_compressed_mesh(hkx: &HkxFile, shape_index: usize) -> HavokResult<()> {
    let shape = &hkx.objects()[shape_index];
    let context = format!("compressed mesh {shape_index}");
    let flags = required_integer(&shape.members, "flags", &context)?;
    let dispatch_type = required_integer(&shape.members, "dispatchType", &context)?;
    if flags != i128::from(FO76_COMPRESSED_MESH_FLAGS)
        || dispatch_type != i128::from(FO76_COMPRESSED_MESH_DISPATCH_TYPE)
    {
        return Err(direct_error(format!(
            "{context} has unsupported source header flags=0x{flags:X} dispatchType={dispatch_type}"
        )));
    }

    let data_index = required_pointer(&shape.members, "data", &context)?;
    let data = hkx.objects().get(data_index).ok_or_else(|| {
        direct_error(format!(
            "compressed mesh {shape_index} data pointer is out of range"
        ))
    })?;
    if data.class_name != "hknpCompressedMeshShapeData" {
        return Err(direct_error(format!(
            "compressed mesh {shape_index} points to {}",
            data.class_name
        )));
    }
    Ok(())
}

fn validate_polytope(hkx: &HkxFile, shape_index: usize) -> HavokResult<()> {
    let shape = hkx
        .objects()
        .get(shape_index)
        .ok_or_else(|| direct_error(format!("polytope {shape_index} is out of range")))?;
    let context = format!("polytope {shape_index}");
    required_integer(&shape.members, "flags", &context)?;
    required_integer(&shape.members, "dispatchType", &context)?;
    match member_value(&shape.members, "convexRadius") {
        Some(HkxValue::F32(radius)) if radius.is_finite() && *radius >= 0.0 => {}
        _ => {
            return Err(direct_error(format!(
                "{context} has an invalid convexRadius"
            )));
        }
    }

    match member_value(&shape.members, "properties") {
        Some(HkxValue::Pointer(Some(properties_index))) => {
            let properties = hkx.objects().get(*properties_index).ok_or_else(|| {
                direct_error(format!(
                    "{context} properties pointer {properties_index} is out of range"
                ))
            })?;
            if properties.class_name != "hkRefCountedProperties" {
                return Err(direct_error(format!(
                    "{context} properties point to {}",
                    properties.class_name
                )));
            }
        }
        Some(HkxValue::Pointer(None)) if shape.class_name == "hknpCapsuleShape" => {}
        _ => {
            return Err(direct_error(format!(
                "{context} has no resolved properties pointer"
            )));
        }
    }

    let vertices = member_array(shape, "vertices")?;
    if vertices.len() < 4 || vertices.len() % 4 != 0 {
        return Err(direct_error(format!(
            "{context} has {} vertices; FO4 requires at least four and a multiple of four",
            vertices.len()
        )));
    }
    for (vertex_index, vertex) in vertices.iter().enumerate() {
        match vertex {
            HkxValue::F32List(values)
                if values.len() == 4 && values.iter().all(|value| value.is_finite()) => {}
            _ => {
                return Err(direct_error(format!(
                    "{context} vertex {vertex_index} is not four finite floats"
                )));
            }
        }
    }

    let planes = member_array(shape, "planes")?;
    let faces = member_array(shape, "faces")?;
    let indices = member_array(shape, "indices")?;
    if planes.len() < 4 || faces.len() < 4 {
        return Err(direct_error(format!(
            "{context} has too few planes ({}) or faces ({})",
            planes.len(),
            faces.len()
        )));
    }
    if planes.len() != faces.len() && planes.len() != faces.len() + 2 {
        return Err(direct_error(format!(
            "{context} has {} planes for {} faces",
            planes.len(),
            faces.len()
        )));
    }
    for (plane_index, plane) in planes.iter().enumerate() {
        match plane {
            HkxValue::F32List(values) if values.len() == 4 => {}
            _ => {
                return Err(direct_error(format!(
                    "{context} plane {plane_index} is not four floats: {plane:?}"
                )));
            }
        }
    }

    let indices = indices
        .iter()
        .enumerate()
        .map(|(index, value)| {
            integer(value)
                .filter(|value| (0..=u8::MAX.into()).contains(value))
                .map(|value| value as usize)
                .ok_or_else(|| direct_error(format!("{context} index {index} is not a byte")))
        })
        .collect::<HavokResult<Vec<_>>>()?;
    if indices.is_empty() {
        return Err(direct_error(format!("{context} has no face indices")));
    }
    for (face_index, face) in faces.iter().enumerate() {
        let members = object_members(face)
            .ok_or_else(|| direct_error(format!("{context} face {face_index} is not an object")))?;
        let first = required_integer(members, "firstIndex", &context)?;
        let count = required_integer(members, "numIndices", &context)?;
        required_integer(members, "minHalfAngle", &context)?;
        if first < 0 || count < 3 {
            return Err(direct_error(format!(
                "{context} face {face_index} has invalid index span {first}+{count}"
            )));
        }
        let first = first as usize;
        let count = count as usize;
        let end = first.checked_add(count).ok_or_else(|| {
            direct_error(format!("{context} face {face_index} index span overflows"))
        })?;
        if end > indices.len() {
            return Err(direct_error(format!(
                "{context} face {face_index} index span {first}..{end} exceeds {} indices",
                indices.len()
            )));
        }
        if indices[first..end]
            .iter()
            .any(|vertex_index| *vertex_index >= vertices.len())
        {
            return Err(direct_error(format!(
                "{context} face {face_index} references a missing vertex"
            )));
        }
    }
    Ok(())
}

fn validate_capsule(hkx: &HkxFile, shape_index: usize) -> HavokResult<()> {
    let shape = hkx
        .objects()
        .get(shape_index)
        .ok_or_else(|| direct_error(format!("capsule {shape_index} is out of range")))?;
    let context = format!("capsule {shape_index}");
    let flags = required_integer(&shape.members, "flags", &context)?;
    let dispatch_type = required_integer(&shape.members, "dispatchType", &context)?;
    if flags != i128::from(FO76_CAPSULE_FLAGS)
        || dispatch_type != i128::from(FO76_CAPSULE_DISPATCH_TYPE)
    {
        return Err(direct_error(format!(
            "{context} has unsupported source header flags=0x{flags:X} dispatchType={dispatch_type}"
        )));
    }

    validate_polytope(hkx, shape_index)?;
    required_floats(&shape.members, "a", 4, &context)?;
    required_floats(&shape.members, "b", 4, &context)?;
    let physical_radius = match member_value(&shape.members, "a") {
        Some(HkxValue::F32List(values)) if values[3] > 0.0 => values[3],
        _ => {
            return Err(direct_error(format!(
                "{context} has an invalid physical radius"
            )));
        }
    };
    match member_value(&shape.members, "convexRadius") {
        Some(HkxValue::F32(radius))
            if radius.is_finite() && *radius >= 0.0 && *radius <= physical_radius => {}
        _ => {
            return Err(direct_error(format!(
                "{context} has an invalid convex radius"
            )));
        }
    }
    if member_array(shape, "vertices")?.len() != 8
        || member_array(shape, "planes")?.len() != 6
        || member_array(shape, "faces")?.len() != 6
        || member_array(shape, "indices")?.len() != 24
    {
        return Err(direct_error(format!(
            "{context} does not carry the expected 8/6/6/24 capsule hull"
        )));
    }
    validate_polytope_support_ids(shape, &context)
}

fn normalize_static_bodies(hkx: &mut HkxFile) -> HavokResult<()> {
    let physics_system = hkx
        .objects_mut()
        .iter_mut()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .ok_or_else(|| direct_error("missing physics system during normalization"))?;
    let bodies = member_array_mut(physics_system, "bodyCinfos")?;
    for body in bodies {
        let members = object_members_mut(body)
            .ok_or_else(|| direct_error("body cinfo is not an inline object"))?;
        set_or_add_member(members, "reservedBodyId", HkxValue::U32(INVALID_ID));
        set_or_add_member(members, "motionId", HkxValue::U32(INVALID_ID));
        set_or_add_member(members, "motionType", HkxValue::U8(0));
        let orientation = member_value(members, "orientation");
        let valid_orientation = matches!(
            orientation,
            Some(HkxValue::F32List(values))
                if values.len() == 4
                    && values.iter().all(|value| value.is_finite())
                    && values.iter().map(|value| value * value).sum::<f32>() > 1.0e-10
        );
        if !valid_orientation {
            set_or_add_member(
                members,
                "orientation",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0]),
            );
        }
    }
    Ok(())
}

fn normalize_compounds(hkx: &mut HkxFile) -> HavokResult<()> {
    let plans = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, object)| object.class_name == "hknpCompoundShape")
        .map(|(shape_index, shape)| {
            let instances = member_object(&shape.members, "instances")
                .and_then(|members| member_array_from_members(members, "elements"))
                .ok_or_else(|| direct_error(format!("compound {shape_index} has no instances")))?;
            let child_sizes = instances
                .iter()
                .enumerate()
                .map(|(instance_index, instance)| {
                    let members = object_members(instance).ok_or_else(|| {
                        direct_error(format!(
                            "compound {shape_index} instance {instance_index} is not an object"
                        ))
                    })?;
                    let child_index = required_pointer(
                        members,
                        "shape",
                        &format!("compound {shape_index} instance {instance_index}"),
                    )?;
                    target_shape_instance_size(hkx, child_index)
                })
                .collect::<HavokResult<Vec<_>>>()?;
            Ok((shape_index, child_sizes))
        })
        .collect::<HavokResult<Vec<_>>>()?;

    for (shape_index, child_sizes) in plans {
        let object = &mut hkx.objects_mut()[shape_index];
        object.class_name = "hknpDynamicCompoundShape".to_string();
        set_or_add_member(
            &mut object.members,
            "flags",
            HkxValue::U16(FO4_COMPOUND_FLAGS),
        );
        set_or_add_member(
            &mut object.members,
            "numShapeKeyBits",
            HkxValue::U8(compound_shape_key_bits(child_sizes.len())),
        );
        set_or_add_member(
            &mut object.members,
            "dispatchType",
            HkxValue::U8(FO4_COMPOUND_DISPATCH_TYPE),
        );
        set_or_add_member(
            &mut object.members,
            "shapeTagCodecInfo",
            HkxValue::U32(u32::MAX),
        );
        set_or_add_member(&mut object.members, "isMutable", HkxValue::Bool(true));

        let instances = member_object_mut(&mut object.members, "instances")
            .and_then(|members| member_array_from_members_mut(members, "elements"))
            .ok_or_else(|| direct_error(format!("compound {shape_index} has no instances")))?;
        for (instance_index, (instance, child_size)) in
            instances.iter_mut().zip(child_sizes).enumerate()
        {
            let members = object_members_mut(instance).ok_or_else(|| {
                direct_error(format!(
                    "compound {shape_index} instance {instance_index} is not an object"
                ))
            })?;
            normalize_shape_instance(members, child_size, shape_index, instance_index)?;
        }
    }
    Ok(())
}

fn normalize_shape_instance(
    members: &mut Vec<HkxMember>,
    child_size: usize,
    compound_index: usize,
    instance_index: usize,
) -> HavokResult<()> {
    if child_size > INT24_MASK as usize {
        return Err(direct_error(format!(
            "compound {compound_index} instance {instance_index} child size {child_size} exceeds int24 capacity"
        )));
    }

    let scale = match member_value(members, "scale") {
        Some(HkxValue::F32List(values)) if values.len() >= 4 => values.clone(),
        _ => {
            return Err(direct_error(format!(
                "compound {compound_index} instance {instance_index} has invalid scale"
            )));
        }
    };
    let transform = match member_value_mut(members, "transform") {
        Some(HkxValue::F32List(values)) if values.len() >= 16 => values,
        _ => {
            return Err(direct_error(format!(
                "compound {compound_index} instance {instance_index} has invalid transform"
            )));
        }
    };

    let source_flags = transform[3].to_bits() & INT24_MASK;
    let mut flags =
        SHAPE_INST_IS_ENABLED | (source_flags & (SHAPE_INST_DEPRECATED | SHAPE_INST_SCALE_SURFACE));
    if transform[12..15].iter().any(|value| value.abs() > 1.0e-6) {
        flags |= SHAPE_INST_HAS_TRANSLATION;
    }
    if [0usize, 1, 2, 4, 5, 6, 8, 9, 10]
        .into_iter()
        .enumerate()
        .any(|(logical_index, transform_index)| {
            let row = logical_index % 3;
            let column = logical_index / 3;
            let identity = if row == column { 1.0 } else { 0.0 };
            (transform[transform_index] - identity).abs() > 1.0e-6
        })
    {
        flags |= SHAPE_INST_HAS_ROTATION;
    }
    if scale[..3].iter().any(|value| (*value - 1.0).abs() > 1.0e-6) {
        flags |= SHAPE_INST_HAS_SCALE;
    }

    let leaf_index = transform[15].to_bits() & INT24_MASK;
    transform[3] = f32::from_bits(pack_inst_row_w(flags));
    transform[7] = 0.0;
    transform[11] = f32::from_bits(INT24_PREFIX | child_size as u32);
    transform[15] = f32::from_bits(INT24_PREFIX | leaf_index);
    set_if_present(members, "isEmpty", HkxValue::U8(0));
    set_if_present(members, "nextEmptyElement", HkxValue::U32(0));
    Ok(())
}

fn target_shape_instance_size(hkx: &HkxFile, shape_index: usize) -> HavokResult<usize> {
    let shape = hkx.objects().get(shape_index).ok_or_else(|| {
        direct_error(format!(
            "compound child shape pointer {shape_index} is out of range"
        ))
    })?;
    match shape.class_name.as_str() {
        "hknpConvexPolytopeShape" => {
            let vertices = member_array(shape, "vertices")?.len();
            let planes = member_array(shape, "planes")?.len();
            let faces = member_array(shape, "faces")?.len();
            let indices = member_array(shape, "indices")?.len();
            Ok(0x50 + vertices * 16 + planes * 16 + align16(faces * 4) + align16(indices))
        }
        "hknpCapsuleShape" => {
            let vertices = member_array(shape, "vertices")?.len();
            let planes = member_array(shape, "planes")?.len();
            let faces = member_array(shape, "faces")?.len();
            let indices = member_array(shape, "indices")?.len();
            Ok(FO4_CAPSULE_BASE_SIZE
                + vertices * 16
                + planes * 16
                + align16(faces * 4)
                + align16(indices))
        }
        "hknpCompressedMeshShape" => Ok(FO4_COMPRESSED_MESH_INSTANCE_SIZE),
        other => Err(direct_error(format!(
            "compound child {shape_index} has unsupported target shape {other}"
        ))),
    }
}

fn compound_shape_key_bits(instance_count: usize) -> u8 {
    let count = instance_count.max(1) as u32;
    (u32::BITS - count.leading_zeros()) as u8
}

fn align16(value: usize) -> usize {
    (value + 15) & !15
}

fn normalize_polytopes(hkx: &mut HkxFile) -> HavokResult<()> {
    for (shape_index, shape) in hkx.objects_mut().iter_mut().enumerate() {
        if shape.class_name != "hknpConvexPolytopeShape" {
            continue;
        }
        validate_polytope_support_ids(shape, &format!("polytope {shape_index}"))?;
        let repaired_planes = repaired_polytope_planes(shape, shape_index)?;

        set_or_add_member(
            &mut shape.members,
            "flags",
            HkxValue::U16(FO4_POLYTOPE_FLAGS),
        );
        set_or_add_member(
            &mut shape.members,
            "dispatchType",
            HkxValue::U8(FO4_POLYTOPE_DISPATCH_TYPE),
        );
        set_or_add_member(&mut shape.members, "numShapeKeyBits", HkxValue::U8(0));

        let face_count = member_array(shape, "faces")?.len();

        let planes = member_array_mut(shape, "planes")?;
        for (plane, repaired) in planes.iter_mut().zip(repaired_planes) {
            if let Some(repaired) = repaired {
                *plane = HkxValue::F32List(repaired.to_vec());
            }
        }
        if planes.len() == face_count {
            planes.extend(
                FO4_POLYTOPE_SENTINEL_PLANES
                    .iter()
                    .map(|plane| HkxValue::F32List(plane.to_vec())),
            );
        } else if planes.len() == face_count + 2 {
            let sentinel_start = planes.len() - 2;
            for (target, plane) in planes[sentinel_start..]
                .iter_mut()
                .zip(FO4_POLYTOPE_SENTINEL_PLANES)
            {
                *target = HkxValue::F32List(plane.to_vec());
            }
        } else {
            return Err(direct_error(format!(
                "polytope {shape_index} has {} planes for {face_count} faces",
                planes.len()
            )));
        }
    }
    Ok(())
}

fn normalize_capsules(hkx: &mut HkxFile) -> HavokResult<()> {
    for (shape_index, shape) in hkx.objects_mut().iter_mut().enumerate() {
        if shape.class_name != "hknpCapsuleShape" {
            continue;
        }
        let context = format!("capsule {shape_index}");
        validate_polytope_support_ids(shape, &context)?;
        set_or_add_member(
            &mut shape.members,
            "flags",
            HkxValue::U16(FO4_CAPSULE_FLAGS),
        );
        set_or_add_member(
            &mut shape.members,
            "dispatchType",
            HkxValue::U8(FO4_CAPSULE_DISPATCH_TYPE),
        );
        set_or_add_member(&mut shape.members, "numShapeKeyBits", HkxValue::U8(0));

        for endpoint in ["a", "b"] {
            match member_value_mut(&mut shape.members, endpoint) {
                Some(HkxValue::F32List(values)) if values.len() == 4 => values[3] = 1.0,
                _ => {
                    return Err(direct_error(format!(
                        "{context} has an invalid {endpoint} endpoint"
                    )));
                }
            }
        }

        let face_count = member_array(shape, "faces")?.len();
        let planes = member_array_mut(shape, "planes")?;
        if planes.len() == face_count {
            planes.extend(
                [FO4_CAPSULE_SENTINEL_PLANE; 2]
                    .into_iter()
                    .map(|plane| HkxValue::F32List(plane.to_vec())),
            );
        } else if planes.len() == face_count + 2 {
            for plane in &mut planes[face_count..] {
                *plane = HkxValue::F32List(FO4_CAPSULE_SENTINEL_PLANE.to_vec());
            }
        } else {
            return Err(direct_error(format!(
                "{context} has {} planes for {face_count} faces",
                planes.len()
            )));
        }

        let index_count = member_array(shape, "indices")?.len();
        let faces = member_array_mut(shape, "faces")?;
        let mut first_index = 0usize;
        for (face_index, face) in faces.iter_mut().enumerate() {
            let members = object_members_mut(face)
                .ok_or_else(|| direct_error(format!("{context} face {face_index} is malformed")))?;
            let count = required_integer(members, "numIndices", &context)?;
            let count = usize::try_from(count).map_err(|_| {
                direct_error(format!(
                    "{context} face {face_index} has a negative index count"
                ))
            })?;
            set_or_add_member(
                members,
                "firstIndex",
                HkxValue::U16(u16::try_from(first_index).map_err(|_| {
                    direct_error(format!("{context} face indices exceed u16 capacity"))
                })?),
            );
            first_index = first_index.checked_add(count).ok_or_else(|| {
                direct_error(format!("{context} face index accumulation overflowed"))
            })?;
        }
        if first_index != index_count {
            return Err(direct_error(format!(
                "{context} faces consume {first_index} of {index_count} indices"
            )));
        }
    }
    Ok(())
}

fn normalize_compressed_mesh_shape_headers(hkx: &mut HkxFile) {
    for shape in hkx
        .objects_mut()
        .iter_mut()
        .filter(|shape| shape.class_name == "hknpCompressedMeshShape")
    {
        set_or_add_member(
            &mut shape.members,
            "flags",
            HkxValue::U16(FO4_COMPRESSED_MESH_FLAGS),
        );
        set_or_add_member(
            &mut shape.members,
            "dispatchType",
            HkxValue::U8(FO4_COMPRESSED_MESH_DISPATCH_TYPE),
        );
    }
}

fn repaired_polytope_planes(
    shape: &HkxObject,
    shape_index: usize,
) -> HavokResult<Vec<Option<[f32; 4]>>> {
    let context = format!("polytope {shape_index}");
    let vertices = member_array(shape, "vertices")?
        .iter()
        .map(|vertex| match vertex {
            HkxValue::F32List(values) if values.len() == 4 => Ok([values[0], values[1], values[2]]),
            _ => Err(direct_error(format!("{context} has a malformed vertex"))),
        })
        .collect::<HavokResult<Vec<_>>>()?;
    let indices = member_array(shape, "indices")?
        .iter()
        .map(|value| {
            integer(value)
                .filter(|value| (0..=u8::MAX.into()).contains(value))
                .map(|value| value as usize)
                .ok_or_else(|| direct_error(format!("{context} has a malformed face index")))
        })
        .collect::<HavokResult<Vec<_>>>()?;
    let faces = member_array(shape, "faces")?;
    let planes = member_array(shape, "planes")?;
    let centroid = vertices.iter().fold([0.0f32; 3], |mut total, vertex| {
        total[0] += vertex[0];
        total[1] += vertex[1];
        total[2] += vertex[2];
        total
    });
    let inverse_count = 1.0 / vertices.len() as f32;
    let centroid = [
        centroid[0] * inverse_count,
        centroid[1] * inverse_count,
        centroid[2] * inverse_count,
    ];

    planes
        .iter()
        .enumerate()
        .map(|(plane_index, plane)| {
            let HkxValue::F32List(values) = plane else {
                return Err(direct_error(format!(
                    "{context} plane {plane_index} is malformed"
                )));
            };
            if values.iter().all(|value| value.is_finite()) {
                return Ok(None);
            }
            let Some(face) = faces.get(plane_index) else {
                return Ok(Some(FO4_POLYTOPE_SENTINEL_PLANES[0]));
            };
            let members = object_members(face).ok_or_else(|| {
                direct_error(format!("{context} face {plane_index} is malformed"))
            })?;
            let first = required_integer(members, "firstIndex", &context)? as usize;
            let count = required_integer(members, "numIndices", &context)? as usize;
            let face_indices = &indices[first..first + count];
            for second in 1..face_indices.len().saturating_sub(1) {
                for third in second + 1..face_indices.len() {
                    let a = vertices[face_indices[0]];
                    let b = vertices[face_indices[second]];
                    let c = vertices[face_indices[third]];
                    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let mut normal = [
                        ab[1] * ac[2] - ab[2] * ac[1],
                        ab[2] * ac[0] - ab[0] * ac[2],
                        ab[0] * ac[1] - ab[1] * ac[0],
                    ];
                    let length =
                        (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                            .sqrt();
                    if !length.is_finite() || length <= 1.0e-8 {
                        continue;
                    }
                    normal[0] /= length;
                    normal[1] /= length;
                    normal[2] /= length;
                    let mut offset = -(normal[0] * a[0] + normal[1] * a[1] + normal[2] * a[2]);
                    if normal[0] * centroid[0]
                        + normal[1] * centroid[1]
                        + normal[2] * centroid[2]
                        + offset
                        > 0.0
                    {
                        normal = [-normal[0], -normal[1], -normal[2]];
                        offset = -offset;
                    }
                    return Ok(Some([normal[0], normal[1], normal[2], offset]));
                }
            }
            Err(direct_error(format!(
                "{context} plane {plane_index} is non-finite and its face is degenerate"
            )))
        })
        .collect()
}

fn normalize_compressed_mass_properties(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hknpShapeMassProperties" {
            continue;
        }
        let Some(compressed) = object
            .members
            .iter_mut()
            .find(|member| member.name == "compressedMassProperties")
            .and_then(|member| member.value.as_object_members_mut())
        else {
            continue;
        };
        for member in compressed {
            if !matches!(
                member.name.as_str(),
                "centerOfMass" | "inertia" | "majorAxisSpace"
            ) {
                continue;
            }
            let values = member
                .value
                .as_object_members()
                .and_then(|members| member_value(members, "values"))
                .and_then(|value| match value {
                    HkxValue::Array(values) => Some(values.clone()),
                    _ => None,
                });
            if let Some(values) = values {
                member.value = HkxValue::Array(values);
            }
        }
    }
}

fn normalize_collision_materials(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if matches!(
            object.class_name.as_str(),
            "hknpCompressedMeshShape"
                | "hknpDynamicCompoundShape"
                | "hknpConvexPolytopeShape"
                | "hknpCapsuleShape"
        ) {
            if let Some(material_crc) = member_value(&object.members, "userData")
                .and_then(integer)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value != 0)
            {
                set_or_add_member(
                    &mut object.members,
                    "userData",
                    HkxValue::U64(u64::from(remap_fo76_collision_material_for_fo4(
                        material_crc,
                    ))),
                );
            }
        }

        if object.class_name != "hknpBSMaterialProperties" {
            continue;
        }
        let Some(materials) = member_array_from_members_mut(&mut object.members, "MaterialA")
        else {
            continue;
        };
        for material in materials {
            let Some(members) = object_members_mut(material) else {
                continue;
            };
            let Some(material_crc) = member_value(members, "uiMaterialCRC")
                .and_then(integer)
                .and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            set_or_add_member(
                members,
                "uiMaterialCRC",
                HkxValue::U32(remap_fo76_collision_material_for_fo4(material_crc)),
            );
        }
    }
}

fn strip_connectivity(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if matches!(
            object.class_name.as_str(),
            "hknpConvexPolytopeShape" | "hknpCapsuleShape"
        ) {
            set_or_add_member(&mut object.members, "connectivity", HkxValue::Pointer(None));
        }
    }
    hkx.retain_objects_remap_pointers(|_, object| {
        object.class_name != "hknpConvexPolytopeShape::Connectivity"
    });
}

fn validate_converted(
    converted_blob: &[u8],
    converted: &HkxFile,
    expected_signature: &PhysicsSignature,
    expected_raw_meshes: &[super::compressed_mesh::RawCompressedMeshData],
    expected_previews: &[Vec<super::preview::PreviewMesh>],
) -> HavokResult<()> {
    if converted.class_version() != 11 || converted.contents_version() != FO4_PACKFILE_VERSION {
        return Err(direct_error(format!(
            "target metadata is class {} contents {}",
            converted.class_version(),
            converted.contents_version()
        )));
    }
    if converted.objects().iter().any(|object| {
        matches!(
            object.class_name.as_str(),
            "hknpCompoundShape" | "hknpConvexPolytopeShape::Connectivity"
        )
    }) {
        return Err(direct_error(
            "target retained a source-only compound or connectivity object",
        ));
    }
    validate_compound_runtime_metadata(converted)?;

    let actual_signature = physics_signature(converted)?;
    compare_physics_signatures(expected_signature, &actual_signature)?;

    let actual_raw_meshes = extract_raw_compressed_meshes_from_blob(converted_blob, None)?;
    compare_raw_meshes(expected_raw_meshes, &actual_raw_meshes)?;
    let actual_previews = body_previews(converted, expected_signature.bodies.len());
    if actual_previews != expected_previews {
        return Err(direct_error("target collision geometry changed"));
    }

    let invariants = Invariants {
        layers: Vec::new(),
        flags: Vec::new(),
        quality_ids: Vec::new(),
        known_body_shape_classes: vec![
            "hknpCompressedMeshShape".to_string(),
            "hknpDynamicCompoundShape".to_string(),
            "hknpConvexPolytopeShape".to_string(),
            "hknpCapsuleShape".to_string(),
        ],
        shape_tag_codec_info: None,
        convex_radius_min: None,
        convex_radius_max: None,
        dynamic_flag_bit: 128,
        invalid_motion_id: i64::from(INVALID_ID),
        min_hull_vertices: 4,
        degenerate_extent_eps: 1.0e-4,
        thin_hull_ratio: 0.01,
    };
    let errors: Vec<_> = validate_objects(converted.objects(), &invariants)
        .into_iter()
        .filter(|violation| violation.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        return Err(direct_error(format!(
            "target failed collision validation: {errors:#?}"
        )));
    }
    for (shape_index, shape) in converted.objects().iter().enumerate() {
        match shape.class_name.as_str() {
            "hknpCompressedMeshShape" => {
                validate_fo4_compressed_mesh_shape_header(shape, shape_index)?;
            }
            "hknpConvexPolytopeShape" => validate_fo4_polytope(shape, shape_index)?,
            "hknpCapsuleShape" => validate_fo4_capsule(shape, shape_index)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_compound_runtime_metadata(hkx: &HkxFile) -> HavokResult<()> {
    for (shape_index, shape) in hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, object)| object.class_name == "hknpDynamicCompoundShape")
    {
        let instances_container = member_object(&shape.members, "instances")
            .ok_or_else(|| direct_error(format!("compound {shape_index} has no instances")))?;
        let instances = member_array_from_members(instances_container, "elements")
            .ok_or_else(|| direct_error(format!("compound {shape_index} has no elements")))?;
        if optional_integer(instances_container, "firstFree") != Some(-1) {
            return Err(direct_error(format!(
                "compound {shape_index} target instance free list is not closed"
            )));
        }

        let data_index = required_pointer(
            &shape.members,
            "boundingVolumeData",
            &format!("compound {shape_index}"),
        )?;
        let data = hkx.objects().get(data_index).ok_or_else(|| {
            direct_error(format!(
                "compound {shape_index} boundingVolumeData pointer is out of range"
            ))
        })?;
        let tree = member_object(&data.members, "aabbTree")
            .ok_or_else(|| direct_error(format!("compound {shape_index} has no AABB tree")))?;
        let nodes = member_array_from_members(tree, "nodes")
            .ok_or_else(|| direct_error(format!("compound {shape_index} tree has no nodes")))?;
        if optional_integer(tree, "numLeaves") != Some(instances.len() as i128) {
            return Err(direct_error(format!(
                "compound {shape_index} tree leaf count does not match its {} instances",
                instances.len()
            )));
        }
        if !instances.is_empty() && optional_integer(tree, "root") != Some(1) {
            return Err(direct_error(format!(
                "compound {shape_index} tree has an invalid root"
            )));
        }
        let first_free = optional_integer(tree, "firstFree")
            .ok_or_else(|| direct_error(format!("compound {shape_index} tree has no firstFree")))?;
        if first_free < 0 || first_free as usize >= nodes.len() {
            return Err(direct_error(format!(
                "compound {shape_index} tree firstFree {first_free} is outside {} nodes",
                nodes.len()
            )));
        }

        for (instance_index, instance) in instances.iter().enumerate() {
            let members = object_members(instance).ok_or_else(|| {
                direct_error(format!(
                    "compound {shape_index} instance {instance_index} is not an object"
                ))
            })?;
            let transform = match member_value(members, "transform") {
                Some(HkxValue::F32List(values)) if values.len() >= 16 => values,
                _ => {
                    return Err(direct_error(format!(
                        "compound {shape_index} instance {instance_index} has no target transform"
                    )));
                }
            };
            let flags = transform[3].to_bits();
            if flags & 0xff00_0000 != INT24_PREFIX
                || flags & SHAPE_INST_IS_ENABLED == 0
                || transform[7].to_bits() != 0
            {
                return Err(direct_error(format!(
                    "compound {shape_index} instance {instance_index} has invalid FO4 flags"
                )));
            }

            let child_index = required_pointer(
                members,
                "shape",
                &format!("compound {shape_index} instance {instance_index}"),
            )?;
            let expected_size = target_shape_instance_size(hkx, child_index)?;
            let encoded_size = transform[11].to_bits();
            if encoded_size & 0xff00_0000 != INT24_PREFIX
                || encoded_size & INT24_MASK != expected_size as u32
            {
                return Err(direct_error(format!(
                    "compound {shape_index} instance {instance_index} has target child size 0x{:X}, expected 0x{expected_size:X}",
                    encoded_size & INT24_MASK
                )));
            }

            let encoded_leaf = transform[15].to_bits();
            if encoded_leaf & 0xff00_0000 != INT24_PREFIX {
                return Err(direct_error(format!(
                    "compound {shape_index} instance {instance_index} has an invalid leaf encoding"
                )));
            }
            let leaf_index = (encoded_leaf & INT24_MASK) as usize;
            let (_, leaf_data) = tree_node_words(nodes.get(leaf_index).ok_or_else(|| {
                direct_error(format!(
                    "compound {shape_index} instance {instance_index} leaf {leaf_index} is outside {} nodes",
                    nodes.len()
                ))
            })?)
            .ok_or_else(|| {
                direct_error(format!(
                    "compound {shape_index} instance {instance_index} leaf {leaf_index} is malformed"
                ))
            })?;
            if leaf_data & 0xffff != 0 || leaf_data >> 16 != instance_index as u32 {
                return Err(direct_error(format!(
                    "compound {shape_index} instance {instance_index} points at tree node {leaf_index} for leaf {}",
                    leaf_data >> 16
                )));
            }
        }
    }
    Ok(())
}

fn tree_node_words(node: &HkxValue) -> Option<(u32, u32)> {
    let aabb = node
        .as_object_members()
        .and_then(|members| member_value(members, "aabb"))
        .and_then(HkxValue::as_object_members)?;
    let min = match member_value(aabb, "min")? {
        HkxValue::F32List(values) if values.len() >= 4 => values,
        _ => return None,
    };
    let max = match member_value(aabb, "max")? {
        HkxValue::F32List(values) if values.len() >= 4 => values,
        _ => return None,
    };
    Some((min[3].to_bits(), max[3].to_bits()))
}

fn validate_fo4_compressed_mesh_shape_header(
    shape: &HkxObject,
    shape_index: usize,
) -> HavokResult<()> {
    if optional_integer(&shape.members, "flags") != Some(i128::from(FO4_COMPRESSED_MESH_FLAGS))
        || optional_integer(&shape.members, "dispatchType")
            != Some(i128::from(FO4_COMPRESSED_MESH_DISPATCH_TYPE))
    {
        return Err(direct_error(format!(
            "target compressed mesh {shape_index} retained non-FO4 shape header values"
        )));
    }
    Ok(())
}

fn validate_polytope_support_ids(shape: &HkxObject, context: &str) -> HavokResult<()> {
    let vertices = member_array(shape, "vertices")?;
    for (vertex_index, vertex) in vertices.iter().enumerate() {
        let HkxValue::F32List(values) = vertex else {
            return Err(direct_error(format!(
                "{context} vertex {vertex_index} is not a vector"
            )));
        };
        if values.len() != 4 || values[..3].iter().any(|value| !value.is_finite()) {
            return Err(direct_error(format!(
                "{context} vertex {vertex_index} is malformed"
            )));
        }

        let encoded_support_id = values[3].to_bits();
        if encoded_support_id & !INT24_MASK != INT24_PREFIX {
            return Err(direct_error(format!(
                "{context} vertex {vertex_index} has an invalid support id"
            )));
        }
        let support_index = (encoded_support_id & INT24_MASK) as usize;
        let Some(HkxValue::F32List(support_vertex)) = vertices.get(support_index) else {
            return Err(direct_error(format!(
                "{context} vertex {vertex_index} support id {support_index} is out of range"
            )));
        };
        if support_vertex.len() != 4 || values[..3] != support_vertex[..3] {
            return Err(direct_error(format!(
                "{context} vertex {vertex_index} support id {support_index} points to different coordinates"
            )));
        }
    }
    Ok(())
}

fn validate_fo4_polytope(shape: &HkxObject, shape_index: usize) -> HavokResult<()> {
    let context = format!("target polytope {shape_index}");
    if optional_integer(&shape.members, "flags") != Some(i128::from(FO4_POLYTOPE_FLAGS))
        || optional_integer(&shape.members, "dispatchType")
            != Some(i128::from(FO4_POLYTOPE_DISPATCH_TYPE))
        || optional_integer(&shape.members, "numShapeKeyBits") != Some(0)
    {
        return Err(direct_error(format!(
            "{context} retained non-FO4 shape header values"
        )));
    }

    validate_polytope_support_ids(shape, &context)?;

    let planes = member_array(shape, "planes")?;
    let faces = member_array(shape, "faces")?;
    for (plane_index, plane) in planes.iter().enumerate() {
        if !matches!(
            plane,
            HkxValue::F32List(values)
                if values.len() == 4 && values.iter().all(|value| value.is_finite())
        ) {
            return Err(direct_error(format!(
                "{context} plane {plane_index} is not four finite floats"
            )));
        }
    }
    if planes.len() != faces.len() + 2
        || planes[planes.len() - 2] != HkxValue::F32List(FO4_POLYTOPE_SENTINEL_PLANES[0].to_vec())
        || planes[planes.len() - 1] != HkxValue::F32List(FO4_POLYTOPE_SENTINEL_PLANES[1].to_vec())
    {
        return Err(direct_error(format!(
            "{context} does not carry the FO4 sentinel planes"
        )));
    }
    Ok(())
}

fn validate_fo4_capsule(shape: &HkxObject, shape_index: usize) -> HavokResult<()> {
    let context = format!("target capsule {shape_index}");
    if optional_integer(&shape.members, "flags") != Some(i128::from(FO4_CAPSULE_FLAGS))
        || optional_integer(&shape.members, "dispatchType")
            != Some(i128::from(FO4_CAPSULE_DISPATCH_TYPE))
        || optional_integer(&shape.members, "numShapeKeyBits") != Some(0)
    {
        return Err(direct_error(format!(
            "{context} retained non-FO4 shape header values"
        )));
    }
    for endpoint in ["a", "b"] {
        if !matches!(
            member_value(&shape.members, endpoint),
            Some(HkxValue::F32List(values))
                if values.len() == 4
                    && values.iter().all(|value| value.is_finite())
                    && values[3] == 1.0
        ) {
            return Err(direct_error(format!(
                "{context} has an invalid {endpoint} endpoint"
            )));
        }
    }
    validate_polytope_support_ids(shape, &context)?;
    let vertices = member_array(shape, "vertices")?;
    let planes = member_array(shape, "planes")?;
    let faces = member_array(shape, "faces")?;
    let indices = member_array(shape, "indices")?;
    if vertices.len() != 8 || planes.len() != 8 || faces.len() != 6 || indices.len() != 24 {
        return Err(direct_error(format!(
            "{context} does not carry the expected 8/8/6/24 target hull"
        )));
    }
    if planes[6..]
        .iter()
        .any(|plane| *plane != HkxValue::F32List(FO4_CAPSULE_SENTINEL_PLANE.to_vec()))
    {
        return Err(direct_error(format!(
            "{context} does not carry FO4 capsule sentinel planes"
        )));
    }
    Ok(())
}

fn compare_raw_meshes(
    expected: &[super::compressed_mesh::RawCompressedMeshData],
    actual: &[super::compressed_mesh::RawCompressedMeshData],
) -> HavokResult<()> {
    if expected.len() != actual.len() {
        return Err(direct_error(format!(
            "target compressed-mesh count changed from {} to {}",
            expected.len(),
            actual.len()
        )));
    }
    macro_rules! compare_field {
        ($index:expr, $expected:expr, $actual:expr, $field:ident) => {
            if $expected.$field != $actual.$field {
                return Err(direct_error(format!(
                    "target compressed mesh {} field {} changed",
                    $index,
                    stringify!($field)
                )));
            }
        };
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        compare_field!(index, expected, actual, user_data);
        compare_field!(index, expected, actual, edge_welding_map);
        compare_field!(index, expected, actual, quad_is_flat);
        compare_field!(index, expected, actual, triangle_is_interior);
        compare_field!(index, expected, actual, materials);
        compare_field!(index, expected, actual, object_aabb_min);
        compare_field!(index, expected, actual, object_aabb_max);
        compare_field!(index, expected, actual, num_primitive_keys);
        compare_field!(index, expected, actual, bits_per_key);
        compare_field!(index, expected, actual, max_key_value);
        if expected.primitive_stores_is_flat_convex != actual.primitive_stores_is_flat_convex {
            return Err(direct_error(format!(
                "target compressed mesh {index} field primitive_stores_is_flat_convex changed from {} to {}",
                expected.primitive_stores_is_flat_convex, actual.primitive_stores_is_flat_convex
            )));
        }
        compare_field!(index, expected, actual, master_tree_nodes);
        compare_field!(index, expected, actual, sections);
        compare_field!(index, expected, actual, shared_vertices);
    }
    Ok(())
}

fn compare_physics_signatures(
    expected: &PhysicsSignature,
    actual: &PhysicsSignature,
) -> HavokResult<()> {
    if expected.materials != actual.materials {
        return Err(direct_error("target physics materials changed"));
    }
    if expected.bodies.len() != actual.bodies.len() {
        return Err(direct_error(format!(
            "target body count changed from {} to {}",
            expected.bodies.len(),
            actual.bodies.len()
        )));
    }
    for (body_index, (expected_body, actual_body)) in
        expected.bodies.iter().zip(&actual.bodies).enumerate()
    {
        if expected_body.semantics != actual_body.semantics {
            return Err(direct_error(format!(
                "target body {body_index} semantics changed from {:?} to {:?}",
                expected_body.semantics, actual_body.semantics
            )));
        }
        compare_shape_signatures(
            &expected_body.shape,
            &actual_body.shape,
            &format!("body {body_index}"),
        )?;
    }
    Ok(())
}

fn compare_shape_signatures(
    expected: &ShapeSignature,
    actual: &ShapeSignature,
    context: &str,
) -> HavokResult<()> {
    if expected.class_name != actual.class_name {
        return Err(direct_error(format!(
            "target {context} class changed from {} to {}",
            expected.class_name, actual.class_name
        )));
    }
    if expected.semantics != actual.semantics {
        return Err(direct_error(format!(
            "target {context} shape semantics changed from {:?} to {:?}",
            expected.semantics, actual.semantics
        )));
    }
    if expected.properties != actual.properties {
        return Err(direct_error(format!(
            "target {context} shape properties changed from {:?} to {:?}",
            expected.properties, actual.properties
        )));
    }
    if expected.instances.len() != actual.instances.len() {
        return Err(direct_error(format!(
            "target {context} instance count changed from {} to {}",
            expected.instances.len(),
            actual.instances.len()
        )));
    }
    for (instance_index, (expected_instance, actual_instance)) in
        expected.instances.iter().zip(&actual.instances).enumerate()
    {
        let instance_context = format!("{context} instance {instance_index}");
        if expected_instance.semantics != actual_instance.semantics {
            return Err(direct_error(format!(
                "target {instance_context} semantics changed from {:?} to {:?}",
                expected_instance.semantics, actual_instance.semantics
            )));
        }
        compare_shape_signatures(
            &expected_instance.shape,
            &actual_instance.shape,
            &instance_context,
        )?;
    }
    Ok(())
}

fn physics_signature(hkx: &HkxFile) -> HavokResult<PhysicsSignature> {
    let physics_system = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .ok_or_else(|| direct_error("missing physics system"))?;
    let materials = member_value(&physics_system.members, "materials")
        .ok_or_else(|| direct_error("physics system has no materials"))
        .map(|value| canonical_value(hkx, value, Some("materials"), &mut HashSet::new()))?;
    let bodies = member_array(physics_system, "bodyCinfos")?
        .iter()
        .enumerate()
        .map(|(body_index, body)| body_signature(hkx, body_index, body))
        .collect::<HavokResult<Vec<_>>>()?;
    Ok(PhysicsSignature { materials, bodies })
}

fn body_signature(hkx: &HkxFile, body_index: usize, body: &HkxValue) -> HavokResult<BodySignature> {
    let members = object_members(body)
        .ok_or_else(|| direct_error(format!("body {body_index} is not an object")))?;
    let semantics = semantic_members(
        hkx,
        members,
        &[
            "flags",
            "collisionFilterInfo",
            "materialId",
            "qualityId",
            "userData",
            "collisionLookAheadDistance",
            "position",
            "orientation",
            "localFrame",
        ],
    )?;
    let shape_index = required_pointer(members, "shape", &format!("body {body_index}"))?;
    Ok(BodySignature {
        semantics,
        shape: shape_signature(hkx, shape_index, &mut HashSet::new())?,
    })
}

fn shape_signature(
    hkx: &HkxFile,
    shape_index: usize,
    visiting: &mut HashSet<usize>,
) -> HavokResult<ShapeSignature> {
    if !visiting.insert(shape_index) {
        return Err(direct_error(format!(
            "shape graph contains a cycle at {shape_index}"
        )));
    }
    let shape = hkx
        .objects()
        .get(shape_index)
        .ok_or_else(|| direct_error(format!("shape index {shape_index} is out of range")))?;
    let class_name = match shape.class_name.as_str() {
        "hknpCompoundShape" | "hknpDynamicCompoundShape" => "hknpDynamicCompoundShape".to_string(),
        "hknpCompressedMeshShape" | "hknpConvexPolytopeShape" | "hknpCapsuleShape" => {
            shape.class_name.clone()
        }
        other => return Err(direct_error(format!("unsupported shape {other}"))),
    };
    let mut semantics = semantic_members(
        hkx,
        &shape.members,
        &[
            "flags",
            "numShapeKeyBits",
            "convexRadius",
            "userData",
            "shapeTagCodecInfo",
            "aabb",
        ],
    )?;
    if class_name == "hknpDynamicCompoundShape" {
        semantics.push(("dispatchType".to_string(), "i:2".to_string()));
        semantics.push(("isMutable".to_string(), "b:true".to_string()));
    } else {
        semantics.extend(semantic_members(hkx, &shape.members, &["dispatchType"])?);
    }
    if class_name == "hknpCapsuleShape" {
        semantics.extend(semantic_members(hkx, &shape.members, &["a", "b"])?);
    }
    let properties = match member_value(&shape.members, "properties") {
        Some(HkxValue::Pointer(Some(index))) => {
            Some(canonical_object(hkx, *index, &mut HashSet::new()))
        }
        Some(HkxValue::Pointer(None)) | None => None,
        Some(other) => {
            return Err(direct_error(format!(
                "shape {shape_index} has malformed properties {}",
                other.variant_name()
            )));
        }
    };

    let mut instances = Vec::new();
    if class_name == "hknpDynamicCompoundShape" {
        let instance_container = member_object(&shape.members, "instances")
            .ok_or_else(|| direct_error(format!("compound {shape_index} has no instances")))?;
        for instance in member_array_from_members(instance_container, "elements")
            .ok_or_else(|| direct_error(format!("compound {shape_index} has no elements")))?
        {
            let members = object_members(instance)
                .ok_or_else(|| direct_error("compound instance is not an object"))?;
            let child_index = required_pointer(
                members,
                "shape",
                &format!("compound {shape_index} instance"),
            )?;
            instances.push(InstanceSignature {
                semantics: semantic_members(
                    hkx,
                    members,
                    &["transform", "scale", "shapeTag", "destructionTag"],
                )?,
                shape: Box::new(shape_signature(hkx, child_index, visiting)?),
            });
        }
    }
    visiting.remove(&shape_index);
    Ok(ShapeSignature {
        class_name,
        semantics,
        properties,
        instances,
    })
}

fn semantic_members(
    hkx: &HkxFile,
    members: &[HkxMember],
    names: &[&str],
) -> HavokResult<Vec<(String, String)>> {
    names
        .iter()
        .filter_map(|name| {
            member_value(members, name).map(|value| {
                Ok((
                    (*name).to_string(),
                    canonical_value(hkx, value, Some(name), &mut HashSet::new()),
                ))
            })
        })
        .collect()
}

fn canonical_object(hkx: &HkxFile, index: usize, visiting: &mut HashSet<usize>) -> String {
    if !visiting.insert(index) {
        return format!("cycle:{index}");
    }
    let result = match hkx.objects().get(index) {
        Some(object) => {
            let members = object
                .members
                .iter()
                .filter(|member| !matches!(member.name.as_str(), "memSizeAndFlags" | "refCount"))
                .map(|member| {
                    format!(
                        "{}={}",
                        member.name,
                        canonical_value(hkx, &member.value, Some(&member.name), visiting)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{}{{{members}}}", object.class_name)
        }
        None => format!("invalid:{index}"),
    };
    visiting.remove(&index);
    result
}

fn canonical_value(
    hkx: &HkxFile,
    value: &HkxValue,
    member_name: Option<&str>,
    visiting: &mut HashSet<usize>,
) -> String {
    if member_name == Some("name") {
        match value {
            HkxValue::String { is_null: true, .. } | HkxValue::Pointer(None) => {
                return "null".to_string();
            }
            value if integer(value) == Some(0) => return "null".to_string(),
            _ => {}
        }
    }
    if member_name.is_some_and(|name| HALF_MATERIAL_MEMBERS.contains(&name)) {
        let bits = match value {
            HkxValue::Half(value) | HkxValue::F32(value) => Some(f32_to_half(*value)),
            HkxValue::U16(value) => Some(*value),
            HkxValue::I16(value) => Some(*value as u16),
            _ => None,
        };
        if let Some(bits) = bits {
            return format!("half:{bits:04x}");
        }
    }
    if matches!(
        member_name,
        Some("centerOfMass" | "inertia" | "majorAxisSpace")
    ) {
        if let Some(values) = value
            .as_object_members()
            .and_then(|members| member_value(members, "values"))
        {
            return canonical_value(hkx, values, member_name, visiting);
        }
    }
    match value {
        HkxValue::Void => "void".to_string(),
        HkxValue::Bool(value) => format!("b:{value}"),
        value if integer(value).is_some() => format!("i:{}", integer(value).unwrap()),
        HkxValue::F32(value) | HkxValue::Half(value) => format!("f:{:08x}", value.to_bits()),
        HkxValue::F32List(values) => format!(
            "f:[{}]",
            values
                .iter()
                .map(|value| format!("{:08x}", value.to_bits()))
                .collect::<Vec<_>>()
                .join(",")
        ),
        HkxValue::String { value, is_null } => format!("s:{is_null}:{value}"),
        HkxValue::Pointer(Some(index)) => canonical_object(hkx, *index, visiting),
        HkxValue::Pointer(None) => "null".to_string(),
        HkxValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| canonical_value(hkx, value, None, visiting))
                .collect::<Vec<_>>()
                .join(",")
        ),
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => format!(
            "{{{}}}",
            members
                .iter()
                .filter(|member| !matches!(member.name.as_str(), "memSizeAndFlags" | "refCount"))
                .map(|member| format!(
                    "{}={}",
                    member.name,
                    canonical_value(hkx, &member.value, Some(&member.name), visiting)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        HkxValue::PendingPtr(value) => format!("pending:{value}"),
        _ => unreachable!(),
    }
}

fn body_previews(hkx: &HkxFile, body_count: usize) -> Vec<Vec<super::preview::PreviewMesh>> {
    (0..body_count)
        .map(|body_index| extract_preview_meshes_from_hkx(hkx, 1.0, Some(body_index)))
        .collect()
}

fn member_value<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a HkxValue> {
    members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn member_value_mut<'a>(members: &'a mut [HkxMember], name: &str) -> Option<&'a mut HkxValue> {
    members
        .iter_mut()
        .find(|member| member.name == name)
        .map(|member| &mut member.value)
}

fn object_members(value: &HkxValue) -> Option<&[HkxMember]> {
    value.as_object_members()
}

fn object_members_mut(value: &mut HkxValue) -> Option<&mut Vec<HkxMember>> {
    value.as_object_members_mut()
}

fn member_array<'a>(object: &'a HkxObject, name: &str) -> HavokResult<&'a [HkxValue]> {
    member_array_from_members(&object.members, name)
        .ok_or_else(|| direct_error(format!("{} has no {name} array", object.class_name)))
}

fn member_array_mut<'a>(
    object: &'a mut HkxObject,
    name: &str,
) -> HavokResult<&'a mut Vec<HkxValue>> {
    object
        .members
        .iter_mut()
        .find(|member| member.name == name)
        .and_then(|member| match &mut member.value {
            HkxValue::Array(values) => Some(values),
            _ => None,
        })
        .ok_or_else(|| direct_error(format!("{} has no {name} array", object.class_name)))
}

fn member_array_from_members<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a [HkxValue]> {
    member_value(members, name).and_then(|value| match value {
        HkxValue::Array(values) => Some(values.as_slice()),
        _ => None,
    })
}

fn member_array_from_members_mut<'a>(
    members: &'a mut [HkxMember],
    name: &str,
) -> Option<&'a mut Vec<HkxValue>> {
    member_value_mut(members, name).and_then(|value| match value {
        HkxValue::Array(values) => Some(values),
        _ => None,
    })
}

fn member_object<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a [HkxMember]> {
    member_value(members, name).and_then(HkxValue::as_object_members)
}

fn member_object_mut<'a>(
    members: &'a mut [HkxMember],
    name: &str,
) -> Option<&'a mut Vec<HkxMember>> {
    member_value_mut(members, name).and_then(HkxValue::as_object_members_mut)
}

fn require_empty_array_if_present(object: &HkxObject, name: &str) -> HavokResult<()> {
    let Some(value) = member_value(&object.members, name) else {
        return Ok(());
    };
    let HkxValue::Array(values) = value else {
        return Err(direct_error(format!(
            "{} {name} is not an array",
            object.class_name
        )));
    };
    if !values.is_empty() {
        return Err(direct_error(format!(
            "{} {name} must be empty, found {} entries",
            object.class_name,
            values.len()
        )));
    }
    Ok(())
}

fn required_pointer(members: &[HkxMember], name: &str, context: &str) -> HavokResult<usize> {
    match member_value(members, name) {
        Some(HkxValue::Pointer(Some(index))) => Ok(*index),
        _ => Err(direct_error(format!(
            "{context} has no resolved {name} pointer"
        ))),
    }
}

fn required_integer(members: &[HkxMember], name: &str, context: &str) -> HavokResult<i128> {
    member_value(members, name)
        .and_then(integer)
        .ok_or_else(|| direct_error(format!("{context} has no integer {name}")))
}

fn optional_integer(members: &[HkxMember], name: &str) -> Option<i128> {
    member_value(members, name).and_then(integer)
}

fn integer(value: &HkxValue) -> Option<i128> {
    match value {
        HkxValue::Bool(value) => Some(i128::from(*value)),
        HkxValue::I8(value) => Some(i128::from(*value)),
        HkxValue::U8(value) => Some(i128::from(*value)),
        HkxValue::I16(value) => Some(i128::from(*value)),
        HkxValue::U16(value) => Some(i128::from(*value)),
        HkxValue::I32(value) => Some(i128::from(*value)),
        HkxValue::U32(value) => Some(i128::from(*value)),
        HkxValue::I64(value) => Some(i128::from(*value)),
        HkxValue::U64(value) => Some(i128::from(*value)),
        _ => None,
    }
}

fn required_floats(
    members: &[HkxMember],
    name: &str,
    count: usize,
    context: &str,
) -> HavokResult<()> {
    match member_value(members, name) {
        Some(HkxValue::F32List(values))
            if values.len() >= count && values.iter().all(|value| value.is_finite()) =>
        {
            Ok(())
        }
        _ => Err(direct_error(format!(
            "{context} has invalid {name}; expected at least {count} finite floats"
        ))),
    }
}

fn set_or_add_member(members: &mut Vec<HkxMember>, name: &str, value: HkxValue) {
    if let Some(member) = members.iter_mut().find(|member| member.name == name) {
        member.value = value;
    } else {
        members.push(HkxMember {
            name: name.to_string(),
            value,
        });
    }
}

fn set_if_present(members: &mut [HkxMember], name: &str, value: HkxValue) {
    if let Some(member) = members.iter_mut().find(|member| member.name == name) {
        member.value = value;
    }
}

fn direct_error(message: impl Into<String>) -> HavokError {
    HavokError::InvalidInput(format!(
        "direct FO76 static collision transcode: {}",
        message.into()
    ))
}
