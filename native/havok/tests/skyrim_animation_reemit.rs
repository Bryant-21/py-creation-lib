use havok_native::api;
use havok_native::hkx::model::{HkxFile, HkxObject};

const SKYRIM_VERSION: &str = "hk_2010.2.0-r1";

#[test]
fn reemitter_fails_closed_for_non_animation_graphs_and_non_skyrim_sources() {
    let unsupported = HkxFile::from_tagxml(
        8,
        SKYRIM_VERSION,
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hkRootLevelContainer".to_string(),
            members: Vec::new(),
        }],
    )
    .save();
    let error = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&unsupported).unwrap_err();
    assert!(error.to_string().contains("unsupported source graph"));

    let fo4_skeleton = include_bytes!("fixtures/skeleton.hkx");
    let error = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(fo4_skeleton).unwrap_err();
    assert!(error.to_string().contains("expected classversion 8"));
}
