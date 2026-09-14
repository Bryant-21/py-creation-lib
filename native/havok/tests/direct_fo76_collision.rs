use std::path::{Path, PathBuf};

use havok_native::collision::{
    convert_fo76_embedded_static_collision_direct, extract_preview_meshes_from_hkx,
    extract_raw_compressed_meshes_from_hkx, is_supported_fo4_collision_material,
};
use havok_native::hkx::model::{HkxFile, HkxMember, HkxObject};
use havok_native::hkx::types::HkxValue;

const INVALID_ID: i128 = 0x7fff_ffff;

#[test]
fn wrangler_shelter_direct_transcode_preserves_static_collision_graph() {
    let Some(path) = wrangler_fixture() else {
        eprintln!("skipping: extracted Wrangler shelter fixture is not present");
        return;
    };
    let nif = std::fs::read(&path).expect("read Wrangler shelter fixture");
    let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
    assert_eq!(source_blob.len(), 79_984);

    let output = convert_fo76_embedded_static_collision_direct(source_blob)
        .expect("direct shelter collision transcode");
    assert_eq!(
        &output[..8],
        b"\x57\xe0\xe0\x57\x10\xc0\xc0\x10",
        "target must be a packfile"
    );

    let hkx = HkxFile::read(&output).expect("reread target packfile");
    assert_eq!(hkx.class_version(), 11);
    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
    assert_eq!(class_count(&hkx, "hknpPhysicsSystemData"), 1);
    assert_eq!(class_count(&hkx, "hknpCompressedMeshShape"), 10);
    assert_eq!(class_count(&hkx, "hknpCompressedMeshShapeData"), 10);
    assert_eq!(class_count(&hkx, "hknpDynamicCompoundShape"), 2);
    assert_eq!(class_count(&hkx, "hknpCompoundShape"), 0);
    assert_eq!(class_count(&hkx, "hknpConvexPolytopeShape"), 3);
    assert_eq!(class_count(&hkx, "hknpBSMaterialProperties"), 10);
    assert_eq!(
        class_count(&hkx, "hknpConvexPolytopeShape::Connectivity"),
        0
    );
    assert_eq!(extract_raw_compressed_meshes_from_hkx(&hkx, None).len(), 10);
    for shape in hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpCompressedMeshShape")
    {
        assert_eq!(integer_member(&shape.members, "flags"), Some(0x0204));
        assert_eq!(integer_member(&shape.members, "dispatchType"), Some(2));
    }

    let physics_system = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("physics system");
    assert!(array_member(physics_system, "motionCinfos").is_empty());
    let bodies = array_member(physics_system, "bodyCinfos");
    assert_eq!(bodies.len(), 3);

    let mut root_classes = Vec::new();
    for body in bodies {
        let members = body.as_object_members().expect("body cinfo");
        assert_eq!(integer_member(members, "motionId"), Some(INVALID_ID));
        assert_eq!(integer_member(members, "reservedBodyId"), Some(INVALID_ID));
        let shape_index = pointer_member(members, "shape").expect("body shape");
        root_classes.push(hkx.objects()[shape_index].class_name.as_str());
    }
    assert_eq!(
        root_classes,
        [
            "hknpCompressedMeshShape",
            "hknpDynamicCompoundShape",
            "hknpDynamicCompoundShape"
        ]
    );

    let first_compound = body_shape(&hkx, 1);
    let second_compound = body_shape(&hkx, 2);
    assert_eq!(compound_leaf_classes(&hkx, first_compound).len(), 8);
    assert!(
        compound_leaf_classes(&hkx, first_compound)
            .iter()
            .all(|class_name| *class_name == "hknpCompressedMeshShape")
    );
    assert_eq!(
        compound_leaf_classes(&hkx, second_compound),
        [
            "hknpCompressedMeshShape",
            "hknpConvexPolytopeShape",
            "hknpConvexPolytopeShape",
            "hknpConvexPolytopeShape"
        ]
    );

    let triangle_counts: Vec<_> = (0..3)
        .map(|body_index| {
            extract_preview_meshes_from_hkx(&hkx, 1.0, Some(body_index))
                .iter()
                .map(|mesh| mesh.triangles.len())
                .sum::<usize>()
        })
        .collect();
    assert_eq!(triangle_counts, [695, 3_398, 359]);
}

#[test]
fn haunted_bell_tower_direct_transcode_preserves_capsule_compounds_and_stairs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(
            "extracted/fo76/meshes/atx/architecture/prefabs/atx_haunted_belltower/atx_haunted_belltower_bellwithbutton.nif",
        );
    if !path.is_file() {
        eprintln!("skipping: extracted Haunted Bell Tower fixture is not present");
        return;
    }

    let nif = std::fs::read(&path).expect("read Haunted Bell Tower fixture");
    let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
    let source = HkxFile::read(source_blob).expect("parse source collision");
    let output = convert_fo76_embedded_static_collision_direct(source_blob)
        .expect("direct Haunted Bell Tower collision transcode");
    let converted = HkxFile::read(&output).expect("parse converted collision");

    assert_eq!(class_count(&converted, "hknpPhysicsSystemData"), 1);
    assert_eq!(class_count(&converted, "hknpCompressedMeshShape"), 25);
    assert_eq!(class_count(&converted, "hknpDynamicCompoundShape"), 6);
    assert_eq!(class_count(&converted, "hknpCompoundShape"), 0);
    assert_eq!(class_count(&converted, "hknpConvexPolytopeShape"), 14);
    assert_eq!(class_count(&converted, "hknpCapsuleShape"), 2);

    let source_body_count = array_member(
        source
            .objects()
            .iter()
            .find(|object| object.class_name == "hknpPhysicsSystemData")
            .expect("source physics system"),
        "bodyCinfos",
    )
    .len();
    assert_eq!(source_body_count, 11);
    assert_eq!(
        (0..source_body_count)
            .map(|body_index| {
                extract_preview_meshes_from_hkx(&source, 1.0, Some(body_index))
                    .iter()
                    .map(|mesh| mesh.triangles.len())
                    .sum::<usize>()
            })
            .collect::<Vec<_>>(),
        (0..source_body_count)
            .map(|body_index| {
                extract_preview_meshes_from_hkx(&converted, 1.0, Some(body_index))
                    .iter()
                    .map(|mesh| mesh.triangles.len())
                    .sum::<usize>()
            })
            .collect::<Vec<_>>()
    );

    let stairs = body_shape(&converted, 2);
    assert_eq!(stairs.class_name, "hknpDynamicCompoundShape");
    assert_eq!(compound_leaf_classes(&converted, stairs).len(), 4);

    for capsule in converted
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpCapsuleShape")
    {
        assert_eq!(integer_member(&capsule.members, "flags"), Some(0x01C3));
        assert_eq!(integer_member(&capsule.members, "dispatchType"), Some(1));
        assert_eq!(array_member(capsule, "vertices").len(), 8);
        assert_eq!(array_member(capsule, "planes").len(), 8);
        assert_eq!(array_member(capsule, "faces").len(), 6);
        assert_eq!(array_member(capsule, "indices").len(), 24);
        for endpoint in ["a", "b"] {
            let HkxValue::F32List(values) =
                member(&capsule.members, endpoint).expect("capsule endpoint")
            else {
                panic!("capsule {endpoint} is not a vector");
            };
            assert_eq!(values[3], 1.0);
        }
    }

    let mut capsule_instance_sizes = Vec::new();
    for compound in converted
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpDynamicCompoundShape")
    {
        for instance in compound_instances(compound) {
            let members = instance.as_object_members().expect("shape instance");
            let child_index = pointer_member(members, "shape").expect("instance shape");
            if converted.objects()[child_index].class_name == "hknpCapsuleShape" {
                capsule_instance_sizes
                    .push(instance_transform(instance).expect("instance transform")[11].to_bits());
            }
        }
    }
    assert_eq!(capsule_instance_sizes, vec![0x3F00_01B0; 2]);

    for body_index in 0..source_body_count {
        assert!(matches!(
            body_motion_type(&converted, body_index),
            None | Some(0)
        ));
    }
}

#[test]
fn restricted_area_shelter_direct_transcode_preserves_static_collision_graph() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("extracted/fo76/meshes/setdressing/shelters")
        .join("shelters_restrictedarea/shelters_restrictedarea_mainfloor.nif");
    if !path.is_file() {
        eprintln!("skipping: extracted Restricted Area shelter fixture is not present");
        return;
    }

    let nif = std::fs::read(&path).expect("read Restricted Area shelter fixture");
    let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
    let source = HkxFile::read(source_blob).expect("parse source collision");
    let output = convert_fo76_embedded_static_collision_direct(source_blob)
        .expect("direct Restricted Area shelter collision transcode");
    let converted = HkxFile::read(&output).expect("parse converted collision");

    assert_eq!(converted.class_version(), 11);
    assert_eq!(converted.contents_version(), "hk_2014.1.0-r1");
    assert_eq!(
        array_member(
            converted
                .objects()
                .iter()
                .find(|object| object.class_name == "hknpPhysicsSystemData")
                .expect("converted physics system"),
            "bodyCinfos",
        )
        .len(),
        7
    );
    assert_eq!(
        (0..7)
            .map(|body_index| body_shape(&converted, body_index).class_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "hknpDynamicCompoundShape",
            "hknpDynamicCompoundShape",
            "hknpConvexPolytopeShape",
            "hknpConvexPolytopeShape",
            "hknpConvexPolytopeShape",
            "hknpConvexPolytopeShape",
            "hknpDynamicCompoundShape",
        ]
    );
    assert_eq!(
        (0..7)
            .map(|body_index| {
                extract_preview_meshes_from_hkx(&source, 1.0, Some(body_index))
                    .iter()
                    .map(|mesh| mesh.triangles.len())
                    .sum::<usize>()
            })
            .collect::<Vec<_>>(),
        (0..7)
            .map(|body_index| {
                extract_preview_meshes_from_hkx(&converted, 1.0, Some(body_index))
                    .iter()
                    .map(|mesh| mesh.triangles.len())
                    .sum::<usize>()
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn reported_non_shelter_statics_direct_transcode_preserves_collision_graphs() {
    let fixtures = [
        (
            "extracted/fo76/meshes/vehicles/dragline/vehicle_dragline.nif",
            2,
        ),
        (
            "extracted/fo76/meshes/setdressing/guntherswildwestshowprops/gwws_entrancesign.nif",
            1,
        ),
        (
            "extracted/fo76/meshes/dlc04/setdressing/vendorcart/vendorcart_04.nif",
            1,
        ),
        (
            "extracted/fo76/meshes/architecture/rangerlookouttower/rangertowerbase03.nif",
            9,
        ),
    ];

    for (relative_path, body_count) in fixtures {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(relative_path);
        if !path.is_file() {
            eprintln!("skipping: extracted fixture is not present: {relative_path}");
            continue;
        }

        let nif = std::fs::read(&path).expect("read static NIF fixture");
        let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
        let source = HkxFile::read(source_blob).expect("parse source collision");
        let output =
            convert_fo76_embedded_static_collision_direct(source_blob).unwrap_or_else(|error| {
                panic!("direct static collision transcode {relative_path}: {error}")
            });
        let converted = HkxFile::read(&output).expect("parse converted collision");

        if relative_path.ends_with("rangertowerbase03.nif") {
            assert_eq!(body_motion_type(&source, 0), Some(1));
            assert!(
                matches!(body_motion_type(&converted, 0), None | Some(0)),
                "converted static body must not retain keyframed motion metadata"
            );
        }

        assert_eq!(converted.class_version(), 11);
        assert_eq!(converted.contents_version(), "hk_2014.1.0-r1");
        assert_eq!(
            array_member(
                converted
                    .objects()
                    .iter()
                    .find(|object| object.class_name == "hknpPhysicsSystemData")
                    .expect("converted physics system"),
                "bodyCinfos",
            )
            .len(),
            body_count
        );
        assert_eq!(
            (0..body_count)
                .map(|body_index| {
                    extract_preview_meshes_from_hkx(&source, 1.0, Some(body_index))
                        .iter()
                        .map(|mesh| mesh.triangles.len())
                        .sum::<usize>()
                })
                .collect::<Vec<_>>(),
            (0..body_count)
                .map(|body_index| {
                    extract_preview_meshes_from_hkx(&converted, 1.0, Some(body_index))
                        .iter()
                        .map(|mesh| mesh.triangles.len())
                        .sum::<usize>()
                })
                .collect::<Vec<_>>(),
            "{relative_path}"
        );
        assert_eq!(
            polytope_support_ids(&converted),
            polytope_support_ids(&source),
            "{relative_path}"
        );
        if relative_path.ends_with("vehicle_dragline.nif") {
            assert!(
                polytope_support_ids(&source)
                    .iter()
                    .any(|ids| ids.iter().enumerate().any(|(index, id)| {
                        usize::try_from(id & 0x00ff_ffff).expect("support id") != index
                    })),
                "dragline fixture must exercise padded duplicate support IDs"
            );
        }
    }
}

#[test]
fn vault_76_podium_direct_transcode_normalizes_fo4_polytopes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("extracted/fo76/meshes/setdressing/chargen/chargen_podiumstage01.nif");
    if !path.is_file() {
        eprintln!("skipping: extracted Vault 76 podium fixture is not present");
        return;
    }

    let nif = std::fs::read(&path).expect("read Vault 76 podium fixture");
    let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
    let source = HkxFile::read(source_blob).expect("parse source collision");
    let output = convert_fo76_embedded_static_collision_direct(source_blob)
        .expect("direct Vault 76 podium collision transcode");
    let converted = HkxFile::read(&output).expect("parse converted collision");

    let source_polytopes = source
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpConvexPolytopeShape")
        .collect::<Vec<_>>();
    assert_eq!(source_polytopes.len(), 10);
    assert!(source_polytopes.iter().all(|shape| {
        integer_member(&shape.members, "dispatchType") == Some(2)
            && array_member(shape, "planes").len() == array_member(shape, "faces").len()
    }));

    let converted_polytopes = converted
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpConvexPolytopeShape")
        .collect::<Vec<_>>();
    assert_eq!(converted_polytopes.len(), source_polytopes.len());
    for (source_shape, shape) in source_polytopes.into_iter().zip(converted_polytopes) {
        assert_eq!(integer_member(&shape.members, "flags"), Some(0x0143));
        assert_eq!(integer_member(&shape.members, "dispatchType"), Some(1));

        assert_eq!(
            polytope_shape_support_ids(shape),
            polytope_shape_support_ids(source_shape)
        );

        let planes = array_member(shape, "planes");
        let faces = array_member(shape, "faces");
        assert_eq!(planes.len(), faces.len() + 2);
        assert_eq!(
            planes[planes.len() - 2],
            HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])
        );
        assert_eq!(
            planes[planes.len() - 1],
            HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0])
        );
        let face_angles = |polytope: &HkxObject| {
            array_member(polytope, "faces")
                .iter()
                .map(|face| {
                    integer_member(
                        face.as_object_members().expect("polytope face"),
                        "minHalfAngle",
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(face_angles(shape), face_angles(source_shape));
    }
}

#[test]
fn golf_cart_direct_transcode_populates_fo4_compound_instance_metadata() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("extracted/fo76/meshes/vehicles/golf_cart/vehicle_golf_cart01.nif");
    if !path.is_file() {
        eprintln!("skipping: extracted golf-cart fixture is not present");
        return;
    }

    let nif = std::fs::read(&path).expect("read golf-cart fixture");
    let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
    let source = HkxFile::read(source_blob).expect("parse source collision");
    let output = convert_fo76_embedded_static_collision_direct(source_blob)
        .expect("direct golf-cart collision transcode");
    let converted = HkxFile::read(&output).expect("parse converted collision");

    assert!(physics_material_crcs(&source).is_empty());
    assert_eq!(shape_user_data(&converted), shape_user_data(&source));
    assert!(
        shape_user_data(&converted)
            .into_iter()
            .all(|crc| is_supported_fo4_collision_material(crc as u32))
    );

    let source_compound = body_shape(&source, 0);
    let source_instances = compound_instances(source_compound);
    assert_eq!(source_instances.len(), 20);
    assert!(source_instances.iter().all(|instance| {
        instance_transform(instance).is_some_and(|transform| transform[11].to_bits() == 0)
    }));

    let converted_compound = body_shape(&converted, 0);
    let converted_instances = compound_instances(converted_compound);
    assert_eq!(converted_instances.len(), source_instances.len());
    let tree_nodes = compound_tree_nodes(&converted, converted_compound);
    for (instance_index, instance) in converted_instances.iter().enumerate() {
        let members = instance.as_object_members().expect("shape instance");
        let transform = instance_transform(instance).expect("instance transform");
        assert_eq!(transform[3].to_bits() & 0xff00_0000, 0x3f00_0000);
        assert_ne!(transform[3].to_bits() & 0x40, 0);
        assert_eq!(transform[7].to_bits(), 0);

        let child_index = pointer_member(members, "shape").expect("instance shape");
        let expected_size = expected_fo4_instance_size(&converted.objects()[child_index]);
        assert_eq!(transform[11].to_bits(), 0x3f00_0000 | expected_size as u32);

        let leaf_index = (transform[15].to_bits() & 0x00ff_ffff) as usize;
        let leaf_aabb = member(
            tree_nodes[leaf_index]
                .as_object_members()
                .expect("tree node"),
            "aabb",
        )
        .and_then(HkxValue::as_object_members)
        .expect("tree node AABB");
        let leaf_data = match member(leaf_aabb, "max") {
            Some(HkxValue::F32List(values)) => values[3].to_bits(),
            other => panic!("tree node max is not a vector: {other:?}"),
        };
        assert_eq!(leaf_data, (instance_index as u32) << 16);
    }
}

#[test]
fn direct_transcode_rejects_non_tag0_input() {
    let error = convert_fo76_embedded_static_collision_direct(b"\x57\xe0\xe0\x57\x10\xc0\xc0\x10")
        .expect_err("FO4 packfile must not enter the direct TAG0 path");
    assert!(
        error
            .to_string()
            .contains("input is not an embedded TAG0 tagfile")
    );
}

#[test]
fn direct_transcode_rejects_dynamic_collision_graph() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("extracted/fo76/meshes/setdressing/tireswing/tireswing01.nif");
    if !path.is_file() {
        eprintln!("skipping: extracted dynamic TireSwing fixture is not present");
        return;
    }

    let nif = std::fs::read(&path).expect("read dynamic NIF fixture");
    let source_blob = embedded_tag0(&nif).expect("find the unique embedded TAG0");
    convert_fo76_embedded_static_collision_direct(source_blob)
        .expect_err("dynamic collision graph must remain on reconstruction");
}

fn wrangler_fixture() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("extracted/fo76/meshes/setdressing/shelters")
        .join("shelters_wranglercasino/shelters_wranglercasino_floor1.nif");
    path.is_file().then_some(path)
}

fn embedded_tag0(nif: &[u8]) -> Option<&[u8]> {
    let matches: Vec<_> = nif
        .windows(4)
        .enumerate()
        .filter_map(|(marker, bytes)| {
            if bytes != b"TAG0" || marker < 8 {
                return None;
            }
            let start = marker - 4;
            let packed_size = u32::from_be_bytes(nif[start..marker].try_into().ok()?);
            let size = (packed_size & 0x3fff_ffff) as usize;
            let nif_size = u32::from_le_bytes(nif[start - 4..start].try_into().ok()?) as usize;
            (size == nif_size && start.checked_add(size)? <= nif.len())
                .then_some(&nif[start..start + size])
        })
        .collect();
    (matches.len() == 1).then(|| matches[0])
}

fn class_count(hkx: &HkxFile, class_name: &str) -> usize {
    hkx.objects()
        .iter()
        .filter(|object| object.class_name == class_name)
        .count()
}

fn member<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a HkxValue> {
    members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn array_member<'a>(object: &'a HkxObject, name: &str) -> &'a [HkxValue] {
    match member(&object.members, name) {
        Some(HkxValue::Array(values)) => values,
        other => panic!("expected {name} array, found {other:?}"),
    }
}

fn pointer_member(members: &[HkxMember], name: &str) -> Option<usize> {
    match member(members, name) {
        Some(HkxValue::Pointer(index)) => *index,
        _ => None,
    }
}

fn integer_member(members: &[HkxMember], name: &str) -> Option<i128> {
    match member(members, name)? {
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

fn body_shape(hkx: &HkxFile, body_index: usize) -> &HkxObject {
    let physics_system = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("physics system");
    let body = &array_member(physics_system, "bodyCinfos")[body_index];
    let shape_index = pointer_member(body.as_object_members().unwrap(), "shape").unwrap();
    &hkx.objects()[shape_index]
}

fn body_motion_type(hkx: &HkxFile, body_index: usize) -> Option<i128> {
    let physics_system = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")?;
    let body = array_member(physics_system, "bodyCinfos").get(body_index)?;
    integer_member(body.as_object_members()?, "motionType")
}

fn compound_leaf_classes<'a>(hkx: &'a HkxFile, compound: &HkxObject) -> Vec<&'a str> {
    compound_instances(compound)
        .iter()
        .map(|instance| {
            let shape_index =
                pointer_member(instance.as_object_members().unwrap(), "shape").unwrap();
            hkx.objects()[shape_index].class_name.as_str()
        })
        .collect()
}

fn compound_instances(compound: &HkxObject) -> &[HkxValue] {
    let instances = member(&compound.members, "instances")
        .and_then(HkxValue::as_object_members)
        .expect("instances");
    match member(instances, "elements") {
        Some(HkxValue::Array(values)) => values,
        _ => panic!("missing instance elements"),
    }
}

fn instance_transform(instance: &HkxValue) -> Option<&[f32]> {
    match member(instance.as_object_members()?, "transform")? {
        HkxValue::F32List(values) if values.len() >= 16 => Some(values),
        _ => None,
    }
}

fn compound_tree_nodes<'a>(hkx: &'a HkxFile, compound: &HkxObject) -> &'a [HkxValue] {
    let data_index =
        pointer_member(&compound.members, "boundingVolumeData").expect("compound backing data");
    let tree = member(&hkx.objects()[data_index].members, "aabbTree")
        .and_then(HkxValue::as_object_members)
        .expect("compound AABB tree");
    match member(tree, "nodes") {
        Some(HkxValue::Array(values)) => values,
        other => panic!("compound tree nodes are not an array: {other:?}"),
    }
}

fn expected_fo4_instance_size(shape: &HkxObject) -> usize {
    match shape.class_name.as_str() {
        "hknpConvexPolytopeShape" => {
            0x50 + array_member(shape, "vertices").len() * 16
                + array_member(shape, "planes").len() * 16
                + align16(array_member(shape, "faces").len() * 4)
                + align16(array_member(shape, "indices").len())
        }
        "hknpCapsuleShape" => {
            0x70 + array_member(shape, "vertices").len() * 16
                + array_member(shape, "planes").len() * 16
                + align16(array_member(shape, "faces").len() * 4)
                + align16(array_member(shape, "indices").len())
        }
        "hknpCompressedMeshShape" => 0x90,
        other => panic!("unsupported compound child {other}"),
    }
}

fn align16(value: usize) -> usize {
    (value + 15) & !15
}

fn physics_material_crcs(hkx: &HkxFile) -> Vec<i128> {
    hkx.objects()
        .iter()
        .filter(|object| object.class_name == "hknpBSMaterialProperties")
        .flat_map(|object| array_member(object, "MaterialA"))
        .filter_map(HkxValue::as_object_members)
        .filter_map(|material| integer_member(material, "uiMaterialCRC"))
        .collect()
}

fn shape_user_data(hkx: &HkxFile) -> Vec<i128> {
    hkx.objects()
        .iter()
        .filter(|object| {
            matches!(
                object.class_name.as_str(),
                "hknpCompressedMeshShape"
                    | "hknpCompoundShape"
                    | "hknpDynamicCompoundShape"
                    | "hknpConvexPolytopeShape"
            )
        })
        .filter_map(|shape| integer_member(&shape.members, "userData"))
        .collect()
}

fn polytope_support_ids(hkx: &HkxFile) -> Vec<Vec<u32>> {
    hkx.objects()
        .iter()
        .filter(|object| object.class_name == "hknpConvexPolytopeShape")
        .map(polytope_shape_support_ids)
        .collect()
}

fn polytope_shape_support_ids(shape: &HkxObject) -> Vec<u32> {
    array_member(shape, "vertices")
        .iter()
        .map(|vertex| match vertex {
            HkxValue::F32List(values) if values.len() == 4 => values[3].to_bits(),
            other => panic!("polytope vertex is not a vector: {other:?}"),
        })
        .collect()
}
