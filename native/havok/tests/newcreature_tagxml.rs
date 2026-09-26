use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::tagxml::{
    read_tagxml_string_with_registry, write_tagxml_string_with_registry,
};
use havok_native::hkx::types::HkxValue;

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

fn single_object_packfile(class: &str, signature: &str, param: &str) -> String {
    format!(
        r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="{class}" signature="{signature}">{param}</hkobject></hksection></hkpackfile>"##
    )
}

#[test]
fn external_blend_curve_enum_resolves_symbolically_and_rejects_unknown_symbols() {
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
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let xml = single_object_packfile(
        "CustomTransition",
        "0x12345678",
        r#"<hkparam name="blendCurve">BLEND_CURVE_SMOOTH</hkparam>"#,
    );
    let first = read_tagxml_string_with_registry(&xml, &mut registry)
        .expect("external BlendCurve symbol should resolve");
    assert_eq!(first.objects()[0].members[0].value, HkxValue::I32(0));
    let written = write_tagxml_string_with_registry(&first, &mut registry)
        .expect("external BlendCurve symbol should write");
    assert!(written.contains(">BLEND_CURVE_SMOOTH</hkparam>"));
    assert!(!written.contains(">0</hkparam>"));
    read_tagxml_string_with_registry(&written, &mut registry)
        .expect("written external BlendCurve should parse");

    let unknown = single_object_packfile(
        "hkbBlendingTransitionEffect",
        "0xfd8584fe",
        r#"<hkparam name="blendCurve">BLEND_CURVE_NOT_REAL</hkparam>"#,
    );
    let error = read_tagxml_string_with_registry(&unknown, &mut registry)
        .expect_err("unknown external enum symbol must remain invalid");
    assert!(error.to_string().contains("invalid enum value"));
}

#[test]
fn grouped_and_fixed_arrays_reconcile_with_declared_shape() {
    let dir = temp_classxml_dir("grouped_fixed_arrays");
    for (file, descriptor) in [
        (
            "hkaSkeleton_5.xml",
            "<class name='hkaSkeleton' version='5' signature='0xfec1cedb'><members><member name='referencePose' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_QSTRANSFORM'/></members></class>",
        ),
        (
            "hkaMeshBinding_3.xml",
            "<class name='hkaMeshBinding' version='3' signature='0x32b0ecb6'><members><member name='boneFromSkinMeshTransforms' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_TRANSFORM'/></members></class>",
        ),
        (
            "hkpConvexVerticesShape_0.xml",
            "<class name='hkpConvexVerticesShape' version='0' signature='0xc21c8b5a'><members><member name='rotatedVertices' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_MATRIX3'/></members></class>",
        ),
        (
            "hkpMotion_0.xml",
            "<class name='hkpMotion' version='0' signature='0x4bcb6ebc'><members><member name='deactivationNumInactiveFrames' offset='0' vtype='TYPE_UINT16' arrsize='2'/></members></class>",
        ),
        (
            "hkMotionState_0.xml",
            "<struct name='hkMotionState' version='0' signature='0x92f37d8c'><members><member name='sweptTransform' offset='0' vtype='TYPE_VECTOR4' arrsize='5'/></members></struct>",
        ),
        (
            "hkpConstraintInstance_0.xml",
            "<class name='hkpConstraintInstance' version='0' signature='0xda4ce91e'><members><member name='entities' offset='40' ctype='hkpEntity' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' arrsize='2'/></members></class>",
        ),
    ] {
        write_temp_classxml(&dir, file, descriptor);
    }
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let accepted: [(&str, &str, &str, usize, usize, HkxValue); 5] = [
        (
            "hkaSkeleton",
            "0xfec1cedb",
            r#"<hkparam name="referencePose" numelements="2">(1 2 3)(0 0 0 1)(1 1 1)
               (4 5 6)(0.1 0.2 0.3 0.9)(2 2 2)</hkparam>"#,
            2,
            0,
            HkxValue::F32List(vec![
                1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0,
            ]),
        ),
        (
            "hkaMeshBinding",
            "0x32b0ecb6",
            r#"<hkparam name="boneFromSkinMeshTransforms" numelements="1">(1 0 0)(0 1 0)(0 0 1)(4 5 6)</hkparam>"#,
            1,
            0,
            HkxValue::F32List(vec![
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 5.0, 6.0, 0.0,
            ]),
        ),
        (
            "hkpMotion",
            "0x4bcb6ebc",
            r#"<hkparam name="deactivationNumInactiveFrames">49152 49152</hkparam>"#,
            2,
            1,
            HkxValue::U16(49152),
        ),
        (
            "hkMotionState",
            "0x92f37d8c",
            r#"<hkparam name="sweptTransform">(1 2 3 4) (5 6 7 8) (9 10 11 12) (13 14 15 16) (17 18 19 20)</hkparam>"#,
            5,
            4,
            HkxValue::F32List(vec![17.0, 18.0, 19.0, 20.0]),
        ),
        (
            "hkpConstraintInstance",
            "0xda4ce91e",
            r#"<hkparam name="entities">#0149 #0147</hkparam>"#,
            2,
            1,
            HkxValue::Pointer(Some(146)),
        ),
    ];
    for (class, signature, param, len, index, expected) in accepted {
        let xml = single_object_packfile(class, signature, param);
        let first = read_tagxml_string_with_registry(&xml, &mut registry)
            .unwrap_or_else(|error| panic!("{class}: {error}"));
        let HkxValue::Array(values) = &first.objects()[0].members[0].value else {
            panic!("{class}: expected array");
        };
        assert_eq!(values.len(), len, "{class}");
        assert_eq!(values[index], expected, "{class}");

        let written = write_tagxml_string_with_registry(&first, &mut registry)
            .unwrap_or_else(|error| panic!("{class} write: {error}"));
        let second = read_tagxml_string_with_registry(&written, &mut registry)
            .unwrap_or_else(|error| panic!("{class} reread: {error}"));
        assert_eq!(
            second.objects()[0].members[0],
            first.objects()[0].members[0],
            "{class}"
        );
    }

    for (class, signature, param, message) in [
        (
            "hkaSkeleton",
            "0xfec1cedb",
            r#"<hkparam name="referencePose" numelements="1">(1 2 3)(0 0 0 1)(1 1)</hkparam>"#,
            "cannot reconcile",
        ),
        (
            "hkpConvexVerticesShape",
            "0xc21c8b5a",
            r#"<hkparam name="rotatedVertices" numelements="1">(1 0 0)(0 1 0)(0 0 1)(9 9 9)</hkparam>"#,
            "cannot reconcile",
        ),
        (
            "hkpMotion",
            "0x4bcb6ebc",
            r#"<hkparam name="deactivationNumInactiveFrames">49152</hkparam>"#,
            "fixed array deactivationNumInactiveFrames expected 2 values, got 1",
        ),
        (
            "hkMotionState",
            "0x92f37d8c",
            r#"<hkparam name="sweptTransform">(1 2 3 4) (5 6 7 8) (9 10 11 12) (13 14 15 16)</hkparam>"#,
            "fixed array sweptTransform expected 5 values, got 4",
        ),
        (
            "hkpConstraintInstance",
            "0xda4ce91e",
            r#"<hkparam name="entities">#0149</hkparam>"#,
            "fixed array entities expected 2 values, got 1",
        ),
    ] {
        let xml = single_object_packfile(class, signature, param);
        let error = read_tagxml_string_with_registry(&xml, &mut registry)
            .expect_err(class)
            .to_string();
        assert!(error.contains(message), "{class}: {error}");
    }
}
