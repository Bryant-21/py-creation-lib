use std::path::{Path, PathBuf};

use havok_native::hkx::HkxFile;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::model::{HkxMember, HkxObject};
use havok_native::hkx::tagxml::{read_tagxml_string, write_tagxml_string};
use havok_native::hkx::types::HkxValue;

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../../refs/newcreature/Samples/Sample Behavior - Sentry Machinegun Turret/Export/CharacterAssets/Ragdoll.xml",
    )
}

fn read_fixture() -> Option<String> {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "skip: optional newcreature fixture is absent: {}",
            path.display()
        );
        return None;
    }
    Some(std::fs::read_to_string(path).expect("read newcreature Ragdoll.xml"))
}

fn object<'a>(objects: &'a [HkxObject], class_name: &str) -> &'a HkxObject {
    objects
        .iter()
        .find(|object| object.class_name == class_name)
        .unwrap_or_else(|| panic!("missing {class_name}"))
}

fn member<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("missing member {name}"))
        .value
}

fn array_len(members: &[HkxMember], name: &str) -> usize {
    let HkxValue::Array(values) = member(members, name) else {
        panic!("{name} is not an array");
    };
    values.len()
}

fn object_members<'a>(members: &'a [HkxMember], name: &str) -> &'a [HkxMember] {
    member(members, name)
        .as_object_members()
        .unwrap_or_else(|| panic!("{name} is not an inline object"))
}

fn integer(value: &HkxValue) -> i64 {
    match value {
        HkxValue::I8(value) => i64::from(*value),
        HkxValue::U8(value) => i64::from(*value),
        HkxValue::I16(value) => i64::from(*value),
        HkxValue::U16(value) => i64::from(*value),
        HkxValue::I32(value) => i64::from(*value),
        HkxValue::U32(value) => i64::from(*value),
        HkxValue::I64(value) => *value,
        HkxValue::U64(value) => i64::try_from(*value).expect("integer fits i64"),
        other => panic!("expected integer, got {other:?}"),
    }
}

fn assert_ragdoll_contract(objects: &[HkxObject]) {
    let ragdoll = object(objects, "hkaRagdollInstance");
    assert_eq!(array_len(&ragdoll.members, "rigidBodies"), 8);
    assert_eq!(array_len(&ragdoll.members, "constraints"), 7);
    assert_eq!(array_len(&ragdoll.members, "boneToRigidBodyMap"), 8);

    let physics = object(objects, "hkpPhysicsSystem");
    assert_eq!(array_len(&physics.members, "rigidBodies"), 8);
    assert_eq!(array_len(&physics.members, "constraints"), 7);
    let physics_data = object(objects, "hkpPhysicsData");
    assert_eq!(array_len(&physics_data.members, "systems"), 1);

    let constraint = object(objects, "hkpConstraintInstance");
    assert_eq!(array_len(&constraint.members, "entities"), 2);
    assert!(matches!(
        member(&constraint.members, "data"),
        HkxValue::Pointer(Some(_))
    ));

    let mappers: Vec<_> = objects
        .iter()
        .filter(|object| object.class_name == "hkaSkeletonMapper")
        .collect();
    assert_eq!(mappers.len(), 2);
    for mapper in mappers {
        let mapping = object_members(&mapper.members, "mapping");
        assert_eq!(array_len(mapping, "simpleMappings"), 8);
        assert!(matches!(
            member(mapping, "skeletonA"),
            HkxValue::Pointer(Some(_))
        ));
        assert!(matches!(
            member(mapping, "skeletonB"),
            HkxValue::Pointer(Some(_))
        ));
    }

    let rigid_body = object(objects, "hkpRigidBody");
    let collidable = object_members(&rigid_body.members, "collidable");
    assert!(matches!(
        member(collidable, "shape"),
        HkxValue::Pointer(Some(_))
    ));
    let broad_phase = object_members(collidable, "broadPhaseHandle");
    assert!(matches!(
        member(broad_phase, "collisionFilterInfo"),
        HkxValue::U32(_)
    ));
    let material = object_members(&rigid_body.members, "material");
    assert_eq!(integer(member(material, "responseType")), 1);
    let motion = object_members(&rigid_body.members, "motion");
    assert_eq!(integer(member(motion, "type")), 3);
    let motion_state = object_members(motion, "motionState");
    assert!(
        matches!(member(motion_state, "transform"), HkxValue::F32List(values) if values.len() == 16)
    );

    let list_shape = object(objects, "hkpListShape");
    assert_eq!(array_len(&list_shape.members, "childInfo"), 5);
    let mopp = object(objects, "hkpMoppCode");
    assert_eq!(array_len(&mopp.members, "data"), 67);

    assert_eq!(
        objects
            .iter()
            .filter(|object| object.class_name == "hkpRigidBody")
            .count(),
        8
    );
    assert_eq!(
        objects
            .iter()
            .filter(|object| object.class_name == "hkpShapeInfo")
            .count(),
        8
    );
    assert_eq!(
        objects
            .iter()
            .filter(|object| object.class_name == "hkpConstraintInstance")
            .count(),
        14
    );
    assert_eq!(
        objects
            .iter()
            .filter(|object| object.class_name == "hkpConvexVerticesShape")
            .count(),
        12
    );
}

fn assert_inline_params_have_descriptors(
    node: roxmltree::Node<'_, '_>,
    class_name: &str,
    registry: &mut DescriptorRegistry,
) {
    let templates = registry
        .get_all_members(class_name)
        .unwrap_or_else(|error| panic!("resolve {class_name}: {error}"));
    for param in node
        .children()
        .filter(|child| child.has_tag_name("hkparam"))
    {
        let name = param.attribute("name").expect("hkparam name");
        let template = templates
            .iter()
            .find(|template| template.name == name)
            .unwrap_or_else(|| panic!("{class_name}.{name} has no descriptor"));
        for inline in param
            .children()
            .filter(|child| child.has_tag_name("hkobject"))
        {
            assert!(
                !template.ctype.is_empty(),
                "{class_name}.{name} contains inline objects without a ctype"
            );
            assert_inline_params_have_descriptors(inline, &template.ctype, registry);
        }
    }
}

#[test]
fn sentry_ragdoll_has_descriptor_for_every_explicit_member() {
    let Some(xml) = read_fixture() else {
        return;
    };
    let document = roxmltree::Document::parse(&xml).expect("parse tutorial XML DOM");
    let mut registry = DescriptorRegistry::new();
    for object in document
        .descendants()
        .filter(|node| node.has_tag_name("hkobject") && node.attribute("class").is_some())
    {
        assert_inline_params_have_descriptors(
            object,
            object.attribute("class").expect("top-level class"),
            &mut registry,
        );
    }
}

#[test]
fn sentry_ragdoll_packfile_round_trip_preserves_physics_contract() {
    let Some(xml) = read_fixture() else {
        return;
    };
    let source = read_tagxml_string(&xml).expect("parse tutorial Ragdoll.xml");
    assert_ragdoll_contract(source.objects());

    let packed = source.save();
    let packed_read = HkxFile::read(&packed).expect("reread packed tutorial ragdoll");
    assert_ragdoll_contract(packed_read.objects());

    let roundtrip_xml = write_tagxml_string(&packed_read).expect("emit round-trip TagXML");
    let roundtrip = read_tagxml_string(&roundtrip_xml).expect("parse emitted TagXML");
    assert_ragdoll_contract(roundtrip.objects());
}

#[test]
fn sentry_ragdoll_rejects_unknown_constraint_priority() {
    let invalid = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
<hksection name="__data__"><hkobject name="#0001" class="hkpConstraintInstance" signature="0xda4ce91e">
<hkparam name="priority">PRIORITY_NOT_REAL</hkparam>
</hkobject></hksection></hkpackfile>"##;
    let error = read_tagxml_string(invalid).expect_err("unknown physics enum must fail");
    assert!(error.to_string().contains("invalid enum value"));
}
