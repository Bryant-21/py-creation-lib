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
fn derive_to_hkx_object_produces_correct_members() {
    let obj = TestArrayClass {
        count: 7,
        scale: 2.5,
        weights: vec![0.1, 0.2, 0.3],
        label: "spine".to_string(),
    };

    let hkx = obj.to_hkx_object(Some("#0042".to_string()));

    assert_eq!(hkx.class_name, "TestArrayClass");
    assert_eq!(hkx.signature, 0x12345678u32);
    assert_eq!(hkx.name, Some("#0042".to_string()));

    let count = hkx.members.iter().find(|m| m.name == "count").unwrap();
    assert_eq!(count.value, HkxValue::I32(7));

    let scale = hkx.members.iter().find(|m| m.name == "scale").unwrap();
    assert_eq!(scale.value, HkxValue::F32(2.5));

    let weights = hkx.members.iter().find(|m| m.name == "weights").unwrap();
    assert_eq!(
        weights.value,
        HkxValue::Array(vec![
            HkxValue::F32(0.1),
            HkxValue::F32(0.2),
            HkxValue::F32(0.3),
        ])
    );

    let label = hkx.members.iter().find(|m| m.name == "label").unwrap();
    assert_eq!(
        label.value,
        HkxValue::String {
            value: "spine".to_string(),
            is_null: false
        }
    );
}

#[test]
fn derive_from_hkx_object_round_trips() {
    let original = TestArrayClass {
        count: 7,
        scale: 2.5,
        weights: vec![0.1, 0.2, 0.3],
        label: "spine".to_string(),
    };

    let hkx = original.to_hkx_object(Some("#0042".to_string()));
    let recovered = TestArrayClass::from_hkx_object(&hkx).expect("round-trip succeeds");

    assert_eq!(original, recovered);
}

#[test]
fn derive_from_hkx_object_missing_member_is_error() {
    // Object with only "count" — missing scale, weights, label.
    let hkx = HkxObject {
        name: None,
        offset: 0,
        signature: 0x12345678,
        class_name: "TestArrayClass".to_string(),
        members: vec![HkxMember {
            name: "count".to_string(),
            value: HkxValue::I32(1),
        }],
    };

    let err = TestArrayClass::from_hkx_object(&hkx).expect_err("missing members must error");
    assert!(
        err.contains("scale"),
        "error names the missing field: {err}"
    );
}
