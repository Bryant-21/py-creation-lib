use havok_macros::HkClass;
use havok_native::hkx::model::{HkxMember, HkxObject};
use havok_native::hkx::types::HkxValue;

/// Synthetic Havok class for derive round-trip test.
/// Mirrors the layout pattern of e.g. `hkaBoneAttachment` — a root object
/// carrying a string name and an hkArray of f32 values.
#[derive(Debug, PartialEq, HkClass)]
#[hk_class(name = "TestArrayClass", signature = 0x12345678)]
struct TestArrayClass {
    count: i32,
    scale: f32,
    #[hk_member(kind = "array")]
    weights: Vec<f32>,
    label: String,
}

#[test]
fn derive_maps_members_round_trips_and_names_missing_members() {
    let original = TestArrayClass {
        count: 7,
        scale: 2.5,
        weights: vec![0.1, 0.2, 0.3],
        label: "spine".to_string(),
    };

    let hkx = original.to_hkx_object(Some("#0042".to_string()));
    assert_eq!(hkx.class_name, "TestArrayClass");
    assert_eq!(hkx.signature, 0x12345678u32);
    assert_eq!(hkx.name, Some("#0042".to_string()));
    let values: Vec<_> = hkx
        .members
        .iter()
        .map(|m| (m.name.as_str(), m.value.clone()))
        .collect();
    assert_eq!(
        values,
        vec![
            ("count", HkxValue::I32(7)),
            ("scale", HkxValue::F32(2.5)),
            (
                "weights",
                HkxValue::Array(vec![
                    HkxValue::F32(0.1),
                    HkxValue::F32(0.2),
                    HkxValue::F32(0.3),
                ])
            ),
            (
                "label",
                HkxValue::String {
                    value: "spine".to_string(),
                    is_null: false
                }
            ),
        ]
    );
    assert_eq!(
        TestArrayClass::from_hkx_object(&hkx).expect("round-trip succeeds"),
        original
    );

    let partial = HkxObject {
        name: None,
        offset: 0,
        signature: 0x12345678,
        class_name: "TestArrayClass".to_string(),
        members: vec![HkxMember {
            name: "count".to_string(),
            value: HkxValue::I32(1),
        }],
    };
    let err = TestArrayClass::from_hkx_object(&partial).expect_err("missing members must error");
    assert!(
        err.contains("scale"),
        "error names the missing field: {err}"
    );
}
