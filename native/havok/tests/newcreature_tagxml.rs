use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::hkx::HkxFile;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::tagxml::{
    read_tagxml_string, read_tagxml_string_with_registry, write_tagxml_string,
    write_tagxml_string_with_registry,
};
use havok_native::hkx::types::HkxValue;

const TARGET_CLASS_VERSION: u32 = 11;
const TARGET_CONTENTS_VERSION: &str = "hk_2014.1.0-r1";

fn temp_classxml_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("havok_newcreature_{name}_{stamp}"));
    std::fs::create_dir_all(&dir).expect("create temp classxml dir");
    dir
}

fn write_temp_classxml(dir: &Path, filename: &str, xml: &str) {
    std::fs::write(dir.join(filename), xml).expect("write temp classxml");
}

#[test]
fn resolves_external_blend_curve_enum_without_numeric_replacement() {
    let dir = temp_classxml_dir("external_blend_curve");
    write_temp_classxml(
        &dir,
        "hkbBlendCurveUtils_0.xml",
        "<struct name='hkbBlendCurveUtils' version='0' signature='0x23041af0'>\
            <enums><enum name='BlendCurve'>\
                <enumitem name='BLEND_CURVE_SMOOTH' value='0'/>\
                <enumitem name='BLEND_CURVE_LINEAR' value='1'/>\
            </enum></enums>\
        </struct>",
    );
    write_temp_classxml(
        &dir,
        "hkbBlendingTransitionEffect_2.xml",
        "<class name='hkbBlendingTransitionEffect' version='2' signature='0xfd8584fe'>\
            <members><member name='blendCurve' offset='0' vtype='TYPE_ENUM' \
                vsubtype='TYPE_INT8' etype='BlendCurve'/></members>\
        </class>",
    );
    write_temp_classxml(
        &dir,
        "CustomTransition_0.xml",
        "<class name='CustomTransition' version='0' signature='0x12345678' \
            parent='hkbBlendingTransitionEffect'><members/></class>",
    );
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="CustomTransition" signature="0x12345678"><hkparam name="blendCurve">BLEND_CURVE_SMOOTH</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(xml, &mut registry)
        .expect("external BlendCurve symbol should resolve");
    assert_eq!(first.objects()[0].members[0].value, HkxValue::I32(0));

    let written = write_tagxml_string_with_registry(&first, &mut registry)
        .expect("external BlendCurve symbol should write");
    assert!(written.contains(">BLEND_CURVE_SMOOTH</hkparam>"));
    assert!(!written.contains(">0</hkparam>"));

    read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written external BlendCurve should parse");
}

#[test]
fn rejects_unknown_external_blend_curve_symbol() {
    let dir = temp_classxml_dir("unknown_blend_curve");
    write_temp_classxml(
        &dir,
        "hkbBlendCurveUtils_0.xml",
        "<struct name='hkbBlendCurveUtils' version='0' signature='0x23041af0'>\
            <enums><enum name='BlendCurve'>\
                <enumitem name='BLEND_CURVE_SMOOTH' value='0'/>\
            </enum></enums>\
        </struct>",
    );
    write_temp_classxml(
        &dir,
        "hkbBlendingTransitionEffect_2.xml",
        "<class name='hkbBlendingTransitionEffect' version='2' signature='0xfd8584fe'>\
            <members><member name='blendCurve' offset='0' vtype='TYPE_ENUM' \
                vsubtype='TYPE_INT8' etype='BlendCurve'/></members>\
        </class>",
    );
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkbBlendingTransitionEffect" signature="0xfd8584fe"><hkparam name="blendCurve">BLEND_CURVE_NOT_REAL</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_tagxml_string_with_registry(xml, &mut registry)
        .expect_err("unknown external enum symbol must remain invalid");
    assert!(error.to_string().contains("invalid enum value"));
}

#[test]
fn reconciles_tutorial_qs_transform_groups_into_padded_binary_values() {
    let dir = temp_classxml_dir("qs_transform_groups");
    write_temp_classxml(
        &dir,
        "hkaSkeleton_5.xml",
        "<class name='hkaSkeleton' version='5' signature='0xfec1cedb'>\
            <members><member name='referencePose' offset='0' vtype='TYPE_ARRAY' \
                vsubtype='TYPE_QSTRANSFORM'/></members>\
        </class>",
    );
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkaSkeleton" signature="0xfec1cedb"><hkparam name="referencePose" numelements="2">
        (1 2 3)(0 0 0 1)(1 1 1)
        (4 5 6)(0.1 0.2 0.3 0.9)(2 2 2)
    </hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(xml, &mut registry)
        .expect("tutorial hkQsTransform groups should parse");
    let HkxValue::Array(transforms) = &first.objects()[0].members[0].value else {
        panic!("referencePose should be an array");
    };
    assert_eq!(transforms.len(), 2);
    assert_eq!(
        transforms[0],
        HkxValue::F32List(vec![
            1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0
        ])
    );

    let written = write_tagxml_string_with_registry(&first, &mut registry)
        .expect("write padded hkQsTransform values");
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written hkQsTransform values should parse");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn rejects_malformed_tutorial_qs_transform_groups() {
    let dir = temp_classxml_dir("malformed_qs_transform_groups");
    write_temp_classxml(
        &dir,
        "hkaSkeleton_5.xml",
        "<class name='hkaSkeleton' version='5' signature='0xfec1cedb'>\
            <members><member name='referencePose' offset='0' vtype='TYPE_ARRAY' \
                vsubtype='TYPE_QSTRANSFORM'/></members>\
        </class>",
    );
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkaSkeleton" signature="0xfec1cedb"><hkparam name="referencePose" numelements="1">(1 2 3)(0 0 0 1)(1 1)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_tagxml_string_with_registry(xml, &mut registry)
        .expect_err("malformed hkQsTransform groups must remain invalid");
    assert!(error.to_string().contains("cannot reconcile"));
}

#[test]
fn reconciles_tutorial_transform_xyz_groups_into_padded_binary_values() {
    let dir = temp_classxml_dir("transform_groups");
    write_temp_classxml(
        &dir,
        "hkaMeshBinding_3.xml",
        "<class name='hkaMeshBinding' version='3' signature='0x32b0ecb6'>\
            <members><member name='boneFromSkinMeshTransforms' offset='0' vtype='TYPE_ARRAY' \
                vsubtype='TYPE_TRANSFORM'/></members>\
        </class>",
    );
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkaMeshBinding" signature="0x32b0ecb6"><hkparam name="boneFromSkinMeshTransforms" numelements="1">(1 0 0)(0 1 0)(0 0 1)(4 5 6)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(xml, &mut registry)
        .expect("tutorial hkTransform groups should parse");
    let HkxValue::Array(transforms) = &first.objects()[0].members[0].value else {
        panic!("bone transforms should be an array");
    };
    assert_eq!(
        transforms[0],
        HkxValue::F32List(vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 5.0, 6.0, 0.0,
        ])
    );

    let written = write_tagxml_string_with_registry(&first, &mut registry)
        .expect("write padded hkTransform values");
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written hkTransform values should parse");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

fn write_matrix3_array_descriptor(dir: &Path) {
    write_temp_classxml(
        dir,
        "hkpConvexVerticesShape_0.xml",
        "<class name='hkpConvexVerticesShape' version='0' signature='0xc21c8b5a'>\
            <members><member name='rotatedVertices' offset='0' vtype='TYPE_ARRAY' \
                vsubtype='TYPE_MATRIX3'/></members>\
        </class>",
    );
}

#[test]
fn reconciles_real_tutorial_rotated_vertices_matrix_count() {
    let path = newcreature_root().join(
        "Samples/Sample Behavior - Sentry Machinegun Turret/Export/CharacterAssets/Ragdoll.xml",
    );
    if !path.exists() {
        eprintln!(
            "skip: optional newcreature corpus file is absent: {}",
            path.display()
        );
        return;
    }

    let corpus_xml = std::fs::read_to_string(&path).expect("read tutorial ragdoll XML");
    let document = roxmltree::Document::parse(&corpus_xml).expect("parse tutorial ragdoll XML");
    let shape = document
        .descendants()
        .find(|node| {
            node.has_tag_name("hkobject")
                && node.attribute("class") == Some("hkpConvexVerticesShape")
        })
        .expect("tutorial ragdoll has a convex vertices shape");
    let rotated_vertices = shape
        .children()
        .find(|node| {
            node.has_tag_name("hkparam") && node.attribute("name") == Some("rotatedVertices")
        })
        .expect("convex vertices shape has rotatedVertices");
    assert_eq!(rotated_vertices.attribute("numelements"), Some("14"));

    let xml = format!(
        r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkpConvexVerticesShape" signature="0xc21c8b5a"><hkparam name="rotatedVertices" numelements="14">{}</hkparam></hkobject></hksection></hkpackfile>"##,
        rotated_vertices.text().unwrap_or("")
    );
    let dir = temp_classxml_dir("real_rotated_vertices");
    write_matrix3_array_descriptor(&dir);
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(&xml, &mut registry)
        .expect("real tutorial rotatedVertices should parse");
    let HkxValue::Array(matrices) = &first.objects()[0].members[0].value else {
        panic!("rotatedVertices should be an array");
    };
    assert_eq!(matrices.len(), 14);
    assert!(
        matrices
            .iter()
            .all(|matrix| matches!(matrix, HkxValue::F32List(values) if values.len() == 12))
    );

    let written = write_tagxml_string_with_registry(&first, &mut registry)
        .expect("write padded rotatedVertices matrices");
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written rotatedVertices matrices should parse");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn rejects_incomplete_tutorial_matrix3_group_count() {
    let dir = temp_classxml_dir("malformed_matrix3_groups");
    write_matrix3_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkpConvexVerticesShape" signature="0xc21c8b5a"><hkparam name="rotatedVertices" numelements="1">(1 0 0)(0 1 0)(0 0 1)(9 9 9)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_tagxml_string_with_registry(xml, &mut registry)
        .expect_err("incomplete repeated Matrix3 groups must remain invalid");
    assert!(error.to_string().contains("cannot reconcile"));
}

fn write_fixed_uint16_array_descriptor(dir: &Path) {
    write_temp_classxml(
        dir,
        "hkpMotion_0.xml",
        "<class name='hkpMotion' version='0' signature='0x4bcb6ebc'>\
            <members><member name='deactivationNumInactiveFrames' offset='0' \
                vtype='TYPE_UINT16' arrsize='2'/></members>\
        </class>",
    );
}

#[test]
fn parses_tutorial_fixed_uint16_array_with_declared_arity() {
    let dir = temp_classxml_dir("fixed_uint16_array");
    write_fixed_uint16_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkpMotion" signature="0x4bcb6ebc"><hkparam name="deactivationNumInactiveFrames">49152 49152</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(xml, &mut registry)
        .expect("fixed UINT16[2] should parse as two values");
    assert_eq!(
        first.objects()[0].members[0].value,
        HkxValue::Array(vec![HkxValue::U16(49152), HkxValue::U16(49152)])
    );

    let written =
        write_tagxml_string_with_registry(&first, &mut registry).expect("write fixed UINT16[2]");
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written fixed UINT16[2] should parse");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn rejects_fixed_scalar_array_with_wrong_declared_arity() {
    let dir = temp_classxml_dir("malformed_fixed_uint16_array");
    write_fixed_uint16_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkpMotion" signature="0x4bcb6ebc"><hkparam name="deactivationNumInactiveFrames">49152</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_tagxml_string_with_registry(xml, &mut registry)
        .expect_err("fixed scalar array arity must match classxml arrsize");
    assert!(
        error
            .to_string()
            .contains("fixed array deactivationNumInactiveFrames expected 2 values, got 1")
    );
}

fn write_fixed_vector4_array_descriptor(dir: &Path) {
    write_temp_classxml(
        dir,
        "hkMotionState_0.xml",
        "<struct name='hkMotionState' version='0' signature='0x92f37d8c'>\
            <members><member name='sweptTransform' offset='0' \
                vtype='TYPE_VECTOR4' arrsize='5'/></members>\
        </struct>",
    );
}

#[test]
fn parses_tutorial_fixed_vector4_array_with_declared_arity() {
    let dir = temp_classxml_dir("fixed_vector4_array");
    write_fixed_vector4_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkMotionState" signature="0x92f37d8c"><hkparam name="sweptTransform">(1 2 3 4) (5 6 7 8) (9 10 11 12) (13 14 15 16) (17 18 19 20)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(xml, &mut registry)
        .expect("fixed Vector4[5] should parse as five values");
    let HkxValue::Array(vectors) = &first.objects()[0].members[0].value else {
        panic!("sweptTransform should be an array");
    };
    assert_eq!(vectors.len(), 5);
    assert_eq!(vectors[4], HkxValue::F32List(vec![17.0, 18.0, 19.0, 20.0]));

    let written =
        write_tagxml_string_with_registry(&first, &mut registry).expect("write fixed Vector4[5]");
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written fixed Vector4[5] should parse");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn rejects_fixed_complex_array_with_wrong_declared_arity() {
    let dir = temp_classxml_dir("malformed_fixed_vector4_array");
    write_fixed_vector4_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkMotionState" signature="0x92f37d8c"><hkparam name="sweptTransform">(1 2 3 4) (5 6 7 8) (9 10 11 12) (13 14 15 16)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_tagxml_string_with_registry(xml, &mut registry)
        .expect_err("fixed complex array arity must match classxml arrsize");
    assert!(
        error
            .to_string()
            .contains("fixed array sweptTransform expected 5 values, got 4")
    );
}

fn write_fixed_pointer_array_descriptor(dir: &Path) {
    write_temp_classxml(
        dir,
        "hkpConstraintInstance_0.xml",
        "<class name='hkpConstraintInstance' version='0' signature='0xda4ce91e'>\
            <members><member name='entities' offset='40' ctype='hkpEntity' \
                vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' arrsize='2'/></members>\
        </class>",
    );
}

#[test]
fn parses_tutorial_fixed_pointer_array_with_declared_arity() {
    let dir = temp_classxml_dir("fixed_pointer_array");
    write_fixed_pointer_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkpConstraintInstance" signature="0xda4ce91e"><hkparam name="entities">#0149 #0147</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let first = read_tagxml_string_with_registry(xml, &mut registry)
        .expect("fixed Pointer[2] should parse as two references");
    assert_eq!(
        first.objects()[0].members[0].value,
        HkxValue::Array(vec![
            HkxValue::Pointer(Some(148)),
            HkxValue::Pointer(Some(146))
        ])
    );

    let written =
        write_tagxml_string_with_registry(&first, &mut registry).expect("write fixed Pointer[2]");
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written fixed Pointer[2] should parse");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn rejects_fixed_pointer_array_with_wrong_declared_arity() {
    let dir = temp_classxml_dir("malformed_fixed_pointer_array");
    write_fixed_pointer_array_descriptor(&dir);
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="hkpConstraintInstance" signature="0xda4ce91e"><hkparam name="entities">#0149</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_tagxml_string_with_registry(xml, &mut registry)
        .expect_err("fixed pointer array arity must match classxml arrsize");
    assert!(
        error
            .to_string()
            .contains("fixed array entities expected 2 values, got 1")
    );
}

fn newcreature_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../refs/newcreature")
}

fn assert_corpus_pack_round_trip(relative: &str) {
    let path = newcreature_root().join(relative);
    if !path.exists() {
        eprintln!(
            "skip: optional newcreature corpus file is absent: {}",
            path.display()
        );
        return;
    }

    let xml = std::fs::read_to_string(&path).expect("read newcreature XML");
    let source = read_tagxml_string(&xml)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    assert_eq!(source.class_version(), TARGET_CLASS_VERSION);
    assert_eq!(source.contents_version(), TARGET_CONTENTS_VERSION);

    let packed = source.save();
    let packed_read = HkxFile::read(&packed)
        .unwrap_or_else(|error| panic!("read packed {}: {error}", path.display()));
    assert_eq!(packed_read.class_version(), TARGET_CLASS_VERSION);
    assert_eq!(packed_read.contents_version(), TARGET_CONTENTS_VERSION);
    assert_eq!(packed_read.objects().len(), source.objects().len());

    let roundtrip_xml = write_tagxml_string(&packed_read)
        .unwrap_or_else(|error| panic!("write round-trip XML {}: {error}", path.display()));
    if xml.contains("BLEND_CURVE_SMOOTH") {
        assert!(
            roundtrip_xml.contains("BLEND_CURVE_SMOOTH"),
            "{} replaced BLEND_CURVE_SMOOTH with a numeric value",
            path.display()
        );
    }
    let roundtrip = read_tagxml_string(&roundtrip_xml)
        .unwrap_or_else(|error| panic!("read round-trip XML {}: {error}", path.display()));
    assert_eq!(roundtrip.class_version(), TARGET_CLASS_VERSION);
    assert_eq!(roundtrip.contents_version(), TARGET_CONTENTS_VERSION);
    assert_eq!(roundtrip.objects().len(), source.objects().len());
}

macro_rules! corpus_test {
    ($name:ident, $relative:literal) => {
        #[test]
        fn $name() {
            assert_corpus_pack_round_trip($relative);
        }
    };
}

corpus_test!(
    packs_example_root_behavior,
    "Samples/ExampleRootBehavior.xml"
);
corpus_test!(
    packs_seeker_mine_root_behavior,
    "Samples/Sample Behavior - Seeker Mine/Export/Behaviors/SeekerMineRootBehavior.xml"
);
corpus_test!(
    packs_seeker_mine_core_behavior,
    "Samples/Sample Behavior - Seeker Mine/Export/Behaviors/SeekerMineCoreBehavior.xml"
);
corpus_test!(
    packs_sentry_turret_root_behavior,
    "Samples/Sample Behavior - Sentry Machinegun Turret/Export/Behaviors/SentryTurretRootBehavior.xml"
);
corpus_test!(
    packs_sentry_turret_core_behavior,
    "Samples/Sample Behavior - Sentry Machinegun Turret/Export/Behaviors/SentryTurretCoreBehavior.xml"
);
corpus_test!(
    packs_sentry_turret_skeleton,
    "Samples/Sample Behavior - Sentry Machinegun Turret/Export/CharacterAssets/Skeleton.xml"
);
corpus_test!(
    packs_sentry_turret_ragdoll,
    "Samples/Sample Behavior - Sentry Machinegun Turret/Export/CharacterAssets/Ragdoll.xml"
);
